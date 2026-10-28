//! 外部登录身份与自动开通 Client Key 的绑定事实

use gateway_core::policy::ClientApiKeyId;

use super::Revision;

/// 外部身份提供方；数据库约束与此枚举保持同一取值集合
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalIdentityProvider {
    Feishu,
}

impl ExternalIdentityProvider {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Feishu => "feishu",
        }
    }
}

/// 提供方返回并经宿主校验的外部用户；只用稳定标识识别身份，姓名仅用于展示
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalIdentity {
    pub provider: ExternalIdentityProvider,
    pub tenant_key: String,
    pub subject: String,
    pub display_name: Option<String>,
}

/// 自动开通在同一事务内的结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalKeyProvision {
    /// 并发登录已先完成绑定，沿用已有 Key
    Existing(ClientApiKeyId),
    Created {
        client_key_id: ClientApiKeyId,
        config_revision: Revision,
    },
    /// 已绑定身份数达到上限，未写入
    LimitReached,
}
