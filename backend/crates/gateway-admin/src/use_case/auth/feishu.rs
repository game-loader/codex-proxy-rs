//! 飞书身份到 Client Key 的解析与首次登录自动开通

use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use gateway_core::{policy::ClientApiKeyId, runtime::SnapshotControl};
use rand_core::{OsRng, RngCore as _};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::{
    FeishuConfig,
    feishu::ProvisionTemplate,
    model::{
        MutationActor, MutationContext,
        auth::{ExternalAuthorization, LoginError},
        client_keys::NewClientKey,
        external_identity::{ExternalIdentity, ExternalIdentityProvider, ExternalKeyProvision},
    },
    ports::external_identity::{
        ExternalIdentityExchange, ExternalIdentityStore, IdentityExchangeError,
    },
    use_case::{client_keys::generate_key, publish_committed},
};

const MAX_NAME_CHARS: usize = 48;

/// 组合根注入的飞书登录能力；未配置时认证服务不提供该登录方式
pub(crate) struct FeishuSignIn {
    config: FeishuConfig,
    template: ProvisionTemplate,
    exchange: Arc<dyn ExternalIdentityExchange>,
    store: Arc<dyn ExternalIdentityStore>,
    snapshot: Arc<dyn SnapshotControl>,
}

impl FeishuSignIn {
    pub(crate) fn new(
        config: FeishuConfig,
        exchange: Arc<dyn ExternalIdentityExchange>,
        store: Arc<dyn ExternalIdentityStore>,
        snapshot: Arc<dyn SnapshotControl>,
    ) -> Result<Self, crate::AdminConfigError> {
        let template = config.validate()?;
        Ok(Self {
            config,
            template,
            exchange,
            store,
            snapshot,
        })
    }

    pub(crate) fn authorization(&self) -> ExternalAuthorization {
        let state = random_token();
        // RFC 7636 verifier 为 43 个 URL 安全字符，challenge 取其 SHA-256
        let code_verifier = random_token();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(code_verifier.as_bytes()));
        ExternalAuthorization {
            url: self.exchange.authorize_url(&state, &challenge),
            state,
            code_verifier,
            secure_callback: self.secure_callback(),
        }
    }

    pub(crate) fn secure_callback(&self) -> bool {
        self.config.redirect_uri.starts_with("https://")
    }

    /// 返回绑定的 Key；Key 的启用状态由调用方按 Key 登录的同一规则检查
    pub(crate) async fn resolve(
        &self,
        code: &str,
        code_verifier: &str,
    ) -> Result<ClientApiKeyId, LoginError> {
        if code.is_empty() || code.len() > 1024 || !(43..=128).contains(&code_verifier.len()) {
            return Err(LoginError::InvalidCredentials);
        }
        let identity = self
            .exchange
            .exchange(code, code_verifier)
            .await
            .map_err(|error| match error {
                IdentityExchangeError::Rejected => LoginError::InvalidCredentials,
                IdentityExchangeError::Unavailable => LoginError::Unavailable,
            })?;
        if !self.config.allows_tenant(&identity.tenant_key) {
            return Err(LoginError::AccessDenied);
        }
        if let Some(key) = self
            .store
            .find_bound_key(identity.provider, &identity.tenant_key, &identity.subject)
            .await
            .map_err(|_| LoginError::Unavailable)?
        {
            return Ok(key);
        }
        if !self.template.enabled {
            return Err(LoginError::AccessDenied);
        }
        self.provision(&identity).await
    }

    async fn provision(&self, identity: &ExternalIdentity) -> Result<ClientApiKeyId, LoginError> {
        let id = ClientApiKeyId::new(format!("key_{}", Uuid::now_v7().simple()))
            .map_err(|_| LoginError::Unavailable)?;
        let key = NewClientKey {
            request_profile_overrides: Default::default(),
            name: key_name(identity, &id),
            label: Some(format!(
                "{}:{}",
                identity.provider.as_str(),
                identity.subject
            )),
            id,
            group_ids: self.template.group_ids.clone(),
            limits: self.template.limits,
            budget: self.template.budget,
            plaintext: generate_key(),
        };
        let context = MutationContext {
            actor: MutationActor::System,
            request_id: format!("feishu-login-{}", Uuid::now_v7().simple()),
        };
        match self
            .store
            .provision_key(identity, key, self.template.max_users, &context)
            .await
            .map_err(|_| LoginError::Unavailable)?
        {
            ExternalKeyProvision::Existing(key) => Ok(key),
            ExternalKeyProvision::Created {
                client_key_id,
                config_revision,
            } => {
                // 事务已提交，发布失败不撤销开通；下次快照发布会带上该 Key
                if publish_committed(self.snapshot.as_ref(), config_revision)
                    .await
                    .is_err()
                {
                    return Err(LoginError::Unavailable);
                }
                Ok(client_key_id)
            }
            ExternalKeyProvision::LimitReached => Err(LoginError::AccessDenied),
        }
    }
}

// 名称在 Key 列表中唯一；附带 ID 片段避免同名成员冲突
fn key_name(identity: &ExternalIdentity, id: &ClientApiKeyId) -> String {
    let suffix = &id.as_str()[id.as_str().len() - 8..];
    let display = identity
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("user");
    let display: String = display
        .chars()
        .filter(|ch| !ch.is_control())
        .take(MAX_NAME_CHARS)
        .collect();
    let provider = match identity.provider {
        ExternalIdentityProvider::Feishu => "feishu",
    };
    format!("{provider}-{display}-{suffix}")
}

fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}
