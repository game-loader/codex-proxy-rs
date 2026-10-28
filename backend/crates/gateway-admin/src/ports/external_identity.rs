//! 外部登录身份的换码与绑定端口；网络协议由 Host 实现，持久化由 Store 实现

use async_trait::async_trait;

use crate::{
    model::{
        MutationContext,
        client_keys::NewClientKey,
        external_identity::{ExternalIdentity, ExternalIdentityProvider, ExternalKeyProvision},
    },
    ports::store::AdminStoreResult,
};
use gateway_core::policy::ClientApiKeyId;

/// 用授权码换取外部用户身份的失败分类
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum IdentityExchangeError {
    /// 授权码无效、过期、已使用或 PKCE 校验失败
    #[error("external authorization was rejected")]
    Rejected,
    /// 网络、超时或提供方暂时故障
    #[error("external identity provider is unavailable")]
    Unavailable,
}

/// 授权码流程的授权地址、换码与用户信息读取
#[async_trait]
pub trait ExternalIdentityExchange: Send + Sync {
    /// 构造带 state 与 S256 PKCE challenge 的授权页地址
    fn authorize_url(&self, state: &str, code_challenge: &str) -> String;

    async fn exchange(
        &self,
        code: &str,
        code_verifier: &str,
    ) -> Result<ExternalIdentity, IdentityExchangeError>;
}

/// 外部身份与 Client Key 的绑定读取和自动开通
#[async_trait]
pub trait ExternalIdentityStore: Send + Sync {
    async fn find_bound_key(
        &self,
        provider: ExternalIdentityProvider,
        tenant_key: &str,
        subject: &str,
    ) -> AdminStoreResult<Option<ClientApiKeyId>>;

    /// 持有配置 revision 行锁复查绑定与人数上限，在同一事务写入 Key、绑定和审计
    async fn provision_key(
        &self,
        identity: &ExternalIdentity,
        key: NewClientKey,
        max_identities: Option<u64>,
        context: &MutationContext,
    ) -> AdminStoreResult<ExternalKeyProvision>;
}
