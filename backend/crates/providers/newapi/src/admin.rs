//! new-api 账号的导入、连接更新、模型目录、导出与本地计价

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use gateway_admin::model::Revision;
use gateway_admin::model::accounts::AccountRecord;
use gateway_admin::model::observability::{
    CalculatedBillingBreakdown, DashboardWireProfile, ProviderBillingInput,
};
use gateway_admin::model::provider_credentials::{
    AuthorizationStarted, CompleteAuthorization, CredentialCommitGuard,
    PendingAuthorizationMutation, PrepareCredentialImport, PrepareCredentialRefresh,
    PrepareCredentialRotation, PreparedAuthorizationCommit, PreparedCredentialCreate,
    PreparedCredentialImport, PreparedCredentialRotation, PreparedCredentialRotationFacts,
    ProviderDocument, ProviderExport, ProviderExportCredentialInput, ProviderModel, ProviderModels,
    ProviderQuota, ProviderQuotaRequest,
};
use gateway_admin::ports::provider::{
    ProviderAdmin, ProviderAdminError, ProviderAdminErrorKind, ProviderUsagePricingInput,
};
use gateway_core::account::{
    CredentialState, OpaqueProviderData, ProviderAccountId, ProviderAccountStore,
};
use gateway_core::error::StoreErrorKind;
use gateway_core::metering::{CalculatedCostBreakdown, ModelPriceOverride};
use gateway_core::operation::{GenerateRequest, Operation, ProtocolPayload};
use gateway_core::routing::{ProviderKind, UpstreamModelId};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::canonical::{RESPONSES_PROTOCOL, billing_breakdown};
use crate::catalog::{CatalogError, NewApiCatalog};
use crate::credential::{AUTHENTICATION_KIND, NewApiCredential};

const MAX_IMPORT_ACCOUNTS: usize = 200;
const MAX_NAME_CHARS: usize = 128;

pub struct NewApiAdminProvider {
    provider_kind: ProviderKind,
    accounts: Arc<dyn ProviderAccountStore>,
    catalog: Arc<NewApiCatalog>,
}

impl NewApiAdminProvider {
    pub fn new(
        provider_kind: ProviderKind,
        accounts: Arc<dyn ProviderAccountStore>,
        catalog: Arc<NewApiCatalog>,
    ) -> Self {
        Self {
            provider_kind,
            accounts,
            catalog,
        }
    }

    fn validate_record(
        &self,
        account: &AccountRecord,
    ) -> Result<ProviderAccountId, ProviderAdminError> {
        if account.provider_kind != self.provider_kind
            || account.authentication_kind != AUTHENTICATION_KIND
        {
            return Err(error(ProviderAdminErrorKind::Invalid));
        }
        ProviderAccountId::new(account.id.clone())
            .map_err(|_| error(ProviderAdminErrorKind::Invalid))
    }
}

struct NoopCommitGuard;

impl CredentialCommitGuard for NoopCommitGuard {
    fn finish(self: Box<Self>) {}
}

#[async_trait]
impl ProviderAdmin for NewApiAdminProvider {
    fn provider_kind(&self) -> &ProviderKind {
        &self.provider_kind
    }

    fn evaluate_usage_pricing(
        &self,
        input: &ProviderUsagePricingInput,
        pricing: Option<&ModelPriceOverride>,
    ) -> Result<Option<CalculatedCostBreakdown>, ProviderAdminError> {
        let usage = &input.usage;
        let (Some(input_tokens), Some(output_tokens)) = (usage.input_tokens, usage.output_tokens)
        else {
            return Ok(None);
        };
        if usage.image_input_tokens.unwrap_or_default() > 0
            || usage.image_output_tokens.unwrap_or_default() > 0
        {
            return Ok(None);
        }
        Ok(billing_breakdown(
            input_tokens,
            output_tokens,
            usage.cached_input_tokens.unwrap_or_default(),
            usage.cache_write_input_tokens.unwrap_or_default(),
            pricing,
        ))
    }

    async fn account_unavailable(&self, _account_id: &ProviderAccountId) {
        self.catalog.invalidate();
    }

    async fn account_facts_changed(&self, _account_ids: &[ProviderAccountId]) {
        self.catalog.invalidate();
    }

    async fn connection_test_operation(
        &self,
        upstream_model: &UpstreamModelId,
        input_text: &str,
    ) -> Result<Operation, ProviderAdminError> {
        let mut body = Map::new();
        body.insert(
            "model".to_owned(),
            Value::String(upstream_model.as_str().to_owned()),
        );
        body.insert(
            "input".to_owned(),
            serde_json::json!([{
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": input_text}]
            }]),
        );
        body.insert("stream".to_owned(), Value::Bool(true));
        body.insert("store".to_owned(), Value::Bool(false));
        let payload = ProtocolPayload::json_object(RESPONSES_PROTOCOL, body)
            .map_err(|_| error(ProviderAdminErrorKind::Invalid))?;
        Ok(Operation::Generate(GenerateRequest::from_protocol_payload(
            payload,
        )))
    }

    fn dashboard_wire_profile(&self) -> Option<DashboardWireProfile> {
        None
    }

    // 历史费用以写入时的快照为准；没有内置价目，不从当前价格反推
    fn calculated_billing(
        &self,
        _input: &ProviderBillingInput,
    ) -> Result<Option<CalculatedBillingBreakdown>, ProviderAdminError> {
        Ok(None)
    }

    async fn prepare_import(
        &self,
        command: PrepareCredentialImport,
    ) -> Result<PreparedCredentialImport, ProviderAdminError> {
        let document = command.document.into_provider_data().into_inner();
        let entries: Vec<Map<String, Value>> = match document.get("accounts") {
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| {
                    item.as_object()
                        .cloned()
                        .ok_or_else(|| error(ProviderAdminErrorKind::Invalid))
                })
                .collect::<Result<_, _>>()?,
            Some(_) => return Err(error(ProviderAdminErrorKind::Invalid)),
            None => vec![document],
        };
        if entries.is_empty() || entries.len() > MAX_IMPORT_ACCOUNTS {
            return Err(error(ProviderAdminErrorKind::Invalid)
                .with_public_message("每次最多导入 200 个 new-api 账号"));
        }
        let observed_at = Utc::now();
        let credentials = entries
            .into_iter()
            .map(|entry| {
                let text = |key: &str| entry.get(key).and_then(Value::as_str).unwrap_or_default();
                let credential =
                    NewApiCredential::new(text("base_url"), text("api_key")).map_err(|_| {
                        error(ProviderAdminErrorKind::Invalid)
                            .with_public_message("new-api 地址或 API Key 不合法")
                    })?;
                let name = account_name(text("name"), &credential)?;
                let account_id =
                    ProviderAccountId::new(format!("acct_{}", Uuid::now_v7().simple()))
                        .map_err(|_| error(ProviderAdminErrorKind::Internal))?;
                Ok(PreparedCredentialCreate {
                    model_access: None,
                    outbound_proxy: command.default_outbound_proxy.clone(),
                    account_id,
                    provider_kind: self.provider_kind.clone(),
                    name,
                    email: None,
                    upstream_user_id: None,
                    upstream_account_id: None,
                    plan_type: None,
                    authentication_kind: AUTHENTICATION_KIND.to_owned(),
                    provider_material: document_of(credential),
                    has_refresh_token: false,
                    access_token_expires_at: None,
                    next_refresh_at: None,
                    enabled: true,
                    credential_state: CredentialState::Ready,
                    credential_observed_at: observed_at,
                })
            })
            .collect::<Result<Vec<_>, ProviderAdminError>>()?;
        Ok(PreparedCredentialImport {
            provider_kind: self.provider_kind.clone(),
            credentials,
        })
    }

    async fn start_authorization(
        &self,
        _pending: PendingAuthorizationMutation,
    ) -> Result<AuthorizationStarted, ProviderAdminError> {
        Err(error(ProviderAdminErrorKind::Unsupported))
    }

    async fn complete_authorization(
        &self,
        _command: CompleteAuthorization,
    ) -> Result<PreparedAuthorizationCommit, ProviderAdminError> {
        Err(error(ProviderAdminErrorKind::Unsupported))
    }

    /// 连接更新只替换提供的地址或密钥，省略的字段沿用当前凭据
    async fn prepare_rotation(
        &self,
        command: PrepareCredentialRotation,
    ) -> Result<PreparedCredentialRotation, ProviderAdminError> {
        let account_id = self.validate_record(&command.account)?;
        let current = self
            .accounts
            .load_current_credential(&account_id)
            .await
            .map_err(map_store_error)?;
        if current.account.revision().get() != command.account.credential_revision.get() {
            return Err(error(ProviderAdminErrorKind::Conflict)
                .with_public_message("账号凭据已被更新，请刷新账号列表后重试"));
        }
        let existing = NewApiCredential::from_document(&current.credential.into_inner())
            .map_err(|_| error(ProviderAdminErrorKind::Invalid))?;
        let material = command.provider_material.into_provider_data().into_inner();
        let text = |key: &str| material.get(key).and_then(Value::as_str);
        let credential = NewApiCredential::new(
            text("base_url").unwrap_or(&existing.base_url),
            text("api_key").unwrap_or(&existing.api_key),
        )
        .map_err(|_| {
            error(ProviderAdminErrorKind::Invalid)
                .with_public_message("new-api 地址或 API Key 不合法")
        })?;
        Ok(PreparedCredentialRotation::new(
            PreparedCredentialRotationFacts {
                account_id,
                provider_kind: self.provider_kind.clone(),
                expected_credential_revision: Revision::new(
                    command.account.credential_revision.get(),
                )
                .map_err(|_| error(ProviderAdminErrorKind::Internal))?,
                replacement_identity: None,
                name: command.account.name.clone(),
                email: None,
                plan_type: None,
                preserve_profile: true,
                preserve_credential_state: false,
                provider_material: document_of(credential),
                has_refresh_token: false,
                access_token_expires_at: None,
                next_refresh_at: None,
            },
            Box::new(NoopCommitGuard),
        ))
    }

    async fn prepare_refresh(
        &self,
        _command: PrepareCredentialRefresh,
    ) -> Result<PreparedCredentialRotation, ProviderAdminError> {
        Err(error(ProviderAdminErrorKind::Unsupported))
    }

    async fn account_configuration(
        &self,
        account_id: &ProviderAccountId,
    ) -> Result<Option<ProviderDocument>, ProviderAdminError> {
        let current = self
            .accounts
            .load_current_credential(account_id)
            .await
            .map_err(map_store_error)?;
        let credential = NewApiCredential::from_document(&current.credential.into_inner())
            .map_err(|_| error(ProviderAdminErrorKind::Invalid))?;
        // 只投影非敏感连接设置，不回显密钥
        let mut document = Map::new();
        document.insert("base_url".to_owned(), Value::String(credential.base_url));
        // 只支持 HTTP/SSE，传输方式固定投影给编辑表单
        document.insert("transport".to_owned(), Value::String("http".to_owned()));
        Ok(Some(ProviderDocument::new(OpaqueProviderData::new(
            document,
        ))))
    }

    // 中转站没有统一的额度接口，账号只展示本地用量
    async fn quota(
        &self,
        _request: ProviderQuotaRequest,
    ) -> Result<ProviderQuota, ProviderAdminError> {
        Ok(ProviderQuota {
            plan_type: None,
            observed_at: None,
            refresh_token_expires_at: None,
            windows: Vec::new(),
            credits: None,
            limit_reached: false,
            provider_data: None,
        })
    }

    async fn models(
        &self,
        account_id: &ProviderAccountId,
        _refresh: bool,
    ) -> Result<ProviderModels, ProviderAdminError> {
        let account = self
            .accounts
            .get_account(account_id)
            .await
            .map_err(map_store_error)?
            .filter(|account| account.provider() == &self.provider_kind)
            .ok_or_else(|| error(ProviderAdminErrorKind::NotFound))?;
        let models = self
            .catalog
            .account_models(&account)
            .await
            .map_err(map_catalog_error)?
            .into_iter()
            .map(|model| {
                Ok(ProviderModel {
                    id: UpstreamModelId::new(model.clone())
                        .map_err(|_| error(ProviderAdminErrorKind::Internal))?,
                    name: model,
                })
            })
            .collect::<Result<Vec<_>, ProviderAdminError>>()?;
        Ok(ProviderModels {
            models,
            observed_at: Some(Utc::now()),
        })
    }

    /// 导出格式与批量导入一致，可直接重新导入
    async fn export_credentials(
        &self,
        credentials: Vec<ProviderExportCredentialInput>,
    ) -> Result<ProviderExport, ProviderAdminError> {
        let mut account_ids = Vec::with_capacity(credentials.len());
        let mut accounts = Vec::with_capacity(credentials.len());
        for input in credentials {
            let account_id = self.validate_record(&input.account)?;
            let credential = NewApiCredential::from_document(
                &input.provider_material.into_provider_data().into_inner(),
            )
            .map_err(|_| error(ProviderAdminErrorKind::Invalid))?;
            accounts.push(serde_json::json!({
                "name": input.account.name,
                "base_url": credential.base_url,
                "api_key": credential.api_key,
            }));
            account_ids.push(account_id);
        }
        let mut document = Map::new();
        document.insert("accounts".to_owned(), Value::Array(accounts));
        Ok(ProviderExport {
            provider_kind: self.provider_kind.clone(),
            account_ids,
            document: ProviderDocument::new(OpaqueProviderData::new(document)),
        })
    }
}

fn account_name(name: &str, credential: &NewApiCredential) -> Result<String, ProviderAdminError> {
    let name = name.trim();
    let name = if name.is_empty() {
        url::Url::parse(&credential.base_url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .unwrap_or_else(|| "new-api".to_owned())
    } else {
        name.to_owned()
    };
    if name.chars().count() > MAX_NAME_CHARS || name.chars().any(char::is_control) {
        return Err(error(ProviderAdminErrorKind::Invalid).with_public_message("账号名称不合法"));
    }
    Ok(name)
}

fn document_of(credential: NewApiCredential) -> ProviderDocument {
    ProviderDocument::new(OpaqueProviderData::new(credential.into_document()))
}

fn error(kind: ProviderAdminErrorKind) -> ProviderAdminError {
    ProviderAdminError::new(kind)
}

fn map_store_error(source: gateway_core::error::StoreError) -> ProviderAdminError {
    error(match source.kind() {
        StoreErrorKind::Conflict => ProviderAdminErrorKind::Conflict,
        StoreErrorKind::InvalidData | StoreErrorKind::InvalidState => {
            ProviderAdminErrorKind::NotFound
        }
        StoreErrorKind::Unavailable => ProviderAdminErrorKind::Unavailable,
        _ => ProviderAdminErrorKind::Internal,
    })
    .with_source(source)
}

fn map_catalog_error(source: CatalogError) -> ProviderAdminError {
    match source {
        CatalogError::Store => error(ProviderAdminErrorKind::Unavailable),
        CatalogError::Upstream | CatalogError::Empty => error(ProviderAdminErrorKind::BadGateway)
            .with_public_message("无法读取上游模型列表，请检查地址和 API Key"),
    }
}
