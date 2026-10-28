//! 上游 `/models` 目录聚合；用于模型发现和路由归属，不作为能力白名单

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use gateway_core::account::{ProviderAccount, ProviderAccountId, ProviderAccountStore};
use gateway_core::routing::{ProviderCatalogGeneration, ProviderKind};
use serde_json::Value;

use crate::credential::{AUTHENTICATION_KIND, NewApiCredential};

const MODELS_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_MODELS_BYTES: usize = 4 * 1024 * 1024;
const MAX_MODELS_PER_ACCOUNT: usize = 2_000;

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("account store is unavailable")]
    Store,
    #[error("upstream model list is unavailable")]
    Upstream,
    #[error("no account returned a model list")]
    Empty,
}

/// 一次聚合结果；模型 ID 到提供它的账号集合
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CatalogSnapshot {
    pub models: BTreeMap<String, BTreeSet<ProviderAccountId>>,
    pub observed_at: Option<DateTime<Utc>>,
}

#[derive(Default)]
struct State {
    generation: u64,
    snapshot: Option<CatalogSnapshot>,
}

pub struct NewApiCatalog {
    provider_kind: ProviderKind,
    accounts: Arc<dyn ProviderAccountStore>,
    state: RwLock<State>,
}

impl NewApiCatalog {
    pub fn new(provider_kind: ProviderKind, accounts: Arc<dyn ProviderAccountStore>) -> Self {
        Self {
            provider_kind,
            accounts,
            state: RwLock::new(State::default()),
        }
    }

    #[must_use]
    pub fn generation(&self) -> ProviderCatalogGeneration {
        ProviderCatalogGeneration::new(self.read().generation)
    }

    /// 账号资格变化后让下一次快照对账重新读取目录
    pub fn invalidate(&self) {
        let mut state = self.write();
        state.generation = state.generation.wrapping_add(1);
    }

    /// 读取全部启用账号的目录；单个账号失败只跳过该账号，全部失败时保留旧目录
    pub async fn synchronize(&self) -> Result<CatalogSnapshot, CatalogError> {
        let accounts = self
            .accounts
            .list_for_provider(&self.provider_kind)
            .await
            .map_err(|_| CatalogError::Store)?;
        let accounts: Vec<ProviderAccount> = accounts
            .into_iter()
            .filter(|account| {
                account.enabled() && account.authentication_kind() == AUTHENTICATION_KIND
            })
            .collect();
        let mut snapshot = CatalogSnapshot {
            models: BTreeMap::new(),
            observed_at: Some(Utc::now()),
        };
        if accounts.is_empty() {
            self.publish(snapshot.clone());
            return Ok(snapshot);
        }
        let reads =
            futures::future::join_all(accounts.iter().map(|account| self.account_models(account)))
                .await;
        let mut succeeded = false;
        for (account, models) in accounts.iter().zip(reads) {
            match models {
                Ok(models) => {
                    succeeded = true;
                    for model in models {
                        snapshot
                            .models
                            .entry(model)
                            .or_default()
                            .insert(account.id().clone());
                    }
                }
                Err(error) => {
                    tracing::warn!(account = account.id().as_str(), error = %error, "new-api model list read failed");
                }
            }
        }
        if !succeeded {
            return Err(CatalogError::Empty);
        }
        self.publish(snapshot.clone());
        Ok(snapshot)
    }

    /// 单账号实时目录，管理端查看模型使用
    pub async fn account_models(
        &self,
        account: &ProviderAccount,
    ) -> Result<Vec<String>, CatalogError> {
        let loaded = self
            .accounts
            .load_credential(account.id(), account.revision())
            .await
            .map_err(|_| CatalogError::Store)?;
        let credential = NewApiCredential::from_document(&loaded.credential.into_inner())
            .map_err(|_| CatalogError::Upstream)?;
        fetch_models(&credential, loaded.account.outbound_proxy()).await
    }

    fn publish(&self, snapshot: CatalogSnapshot) {
        let mut state = self.write();
        let changed = state
            .snapshot
            .as_ref()
            .is_none_or(|current| current.models != snapshot.models);
        if changed {
            state.generation = state.generation.wrapping_add(1);
        }
        state.snapshot = Some(snapshot);
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, State> {
        self.state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, State> {
        self.state
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

pub(crate) async fn fetch_models(
    credential: &NewApiCredential,
    proxy: Option<&gateway_core::account::OutboundProxy>,
) -> Result<Vec<String>, CatalogError> {
    let client = crate::http::client(proxy).map_err(|_| CatalogError::Upstream)?;
    let response = client
        .get(credential.endpoint("models"))
        .bearer_auth(&credential.api_key)
        .timeout(MODELS_TIMEOUT)
        .send()
        .await
        .map_err(|_| CatalogError::Upstream)?;
    if !response.status().is_success() {
        return Err(CatalogError::Upstream);
    }
    let body = crate::http::bounded_body(response, MAX_MODELS_BYTES + 1).await;
    if body.len() > MAX_MODELS_BYTES {
        return Err(CatalogError::Upstream);
    }
    parse_models(&body)
}

/// 解析标准 `{"data":[{"id":...}]}` 列表；忽略不合法的模型 ID
pub fn parse_models(body: &[u8]) -> Result<Vec<String>, CatalogError> {
    let value: Value = serde_json::from_slice(body).map_err(|_| CatalogError::Upstream)?;
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or(CatalogError::Upstream)?;
    let mut models: Vec<String> = data
        .iter()
        .filter_map(|item| item.get("id").and_then(Value::as_str))
        .filter(|id| {
            !id.is_empty()
                && id.len() <= 128
                && !id.chars().any(|ch| ch.is_whitespace() || ch.is_control())
        })
        .map(str::to_owned)
        .collect();
    models.sort();
    models.dedup();
    models.truncate(MAX_MODELS_PER_ACCOUNT);
    Ok(models)
}
