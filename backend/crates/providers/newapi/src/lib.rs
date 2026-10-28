//! new-api 等 OpenAI Responses 兼容中转的独立 Provider
//!
//! 只支持 API Key 账号与 HTTP/SSE，不做会话亲和、原生续接和额度窗口
//! 会话绑定、计价分组和账号调度均以 `newapi` 为键，与 OpenAI Provider 隔离

use std::sync::Arc;

use gateway_admin::ports::provider::ProviderAdmin;
use gateway_core::engine::provider::Provider;
use gateway_core::provider_ports::ProviderStorePorts;
use gateway_core::routing::ProviderKind;

mod admin;
mod canonical;
mod catalog;
mod credential;
mod http;
mod provider;
mod selector;

pub use canonical::{NewApiCanonicalDecoder, billing_breakdown};
pub use catalog::parse_models;
pub use credential::{AUTHENTICATION_KIND, NewApiCredential};
pub use provider::PROVIDER_NAME;

/// 初始化后交给组合根的能力集合
pub struct ProviderBundle {
    core_provider: Arc<dyn Provider>,
    admin_provider: Arc<dyn ProviderAdmin>,
}

impl ProviderBundle {
    #[must_use]
    pub fn core_provider(&self) -> Arc<dyn Provider> {
        Arc::clone(&self.core_provider)
    }

    #[must_use]
    pub fn admin_provider(&self) -> Arc<dyn ProviderAdmin> {
        Arc::clone(&self.admin_provider)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NewApiInitializeError {
    #[error("new-api provider kind is invalid")]
    ProviderKind,
}

/// 构造数据面与管理面实现；不注册后台任务，目录随快照对账刷新
pub fn initialize(ports: &ProviderStorePorts) -> Result<ProviderBundle, NewApiInitializeError> {
    let provider_kind =
        ProviderKind::new(PROVIDER_NAME).map_err(|_| NewApiInitializeError::ProviderKind)?;
    let accounts = ports.accounts();
    let account_feedback = ports.account_feedback();
    let catalog = Arc::new(catalog::NewApiCatalog::new(
        provider_kind.clone(),
        Arc::clone(&accounts),
    ));
    let selector = selector::NewApiSelector::new(
        provider_kind.clone(),
        Arc::clone(&accounts),
        ports.leases(),
        Arc::clone(&account_feedback),
    );
    let core_provider: Arc<dyn Provider> = Arc::new(provider::NewApiProvider::new(
        selector,
        Arc::clone(&catalog),
        account_feedback,
    ));
    let admin_provider: Arc<dyn ProviderAdmin> = Arc::new(admin::NewApiAdminProvider::new(
        provider_kind,
        accounts,
        catalog,
    ));
    Ok(ProviderBundle {
        core_provider,
        admin_provider,
    })
}
