//! 飞书登录的启动配置与首次登录开通模板

use std::fmt;

use gateway_core::{
    engine::budget::ClientBudgetLimits, metering::Decimal, routing::AccountGroupId,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::AdminConfigError;

const MAX_TENANTS: usize = 64;
const MAX_TEXT_BYTES: usize = 512;

/// `client.feishu` 配置段；缺省时不提供飞书登录
#[derive(Clone, Deserialize, PartialEq, Eq)]
pub struct FeishuConfig {
    pub app_id: String,
    pub app_secret: FeishuAppSecret,
    /// 飞书开发者后台“安全设置”中登记的完整回调地址
    pub redirect_uri: String,
    /// 只允许这些企业的成员登录；为空时拒绝全部
    pub allowed_tenant_keys: Vec<String>,
    #[serde(default)]
    pub provision: FeishuProvisionConfig,
}

/// 飞书应用密钥；Debug 不输出原值
#[derive(Clone, Deserialize)]
#[serde(transparent)]
pub struct FeishuAppSecret(SecretString);

impl FeishuAppSecret {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(SecretString::from(value.into()))
    }

    #[must_use]
    pub fn expose(&self) -> &str {
        self.0.expose_secret()
    }
}

impl PartialEq for FeishuAppSecret {
    fn eq(&self, other: &Self) -> bool {
        self.expose() == other.expose()
    }
}

impl Eq for FeishuAppSecret {}

/// 首次登录自动创建 Key 时使用的模板；已开通的 Key 不随模板变化
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct FeishuProvisionConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub group_ids: Vec<String>,
    #[serde(default)]
    pub max_concurrency: u64,
    #[serde(default)]
    pub requests_per_minute: u64,
    /// 美元金额字符串，`0` 表示不限
    #[serde(default = "default_zero")]
    pub daily_limit_usd: String,
    #[serde(default = "default_zero")]
    pub weekly_limit_usd: String,
    /// 自动开通的身份总数上限，缺省不限
    #[serde(default)]
    pub max_users: Option<u64>,
}

impl Default for FeishuProvisionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            group_ids: Vec::new(),
            max_concurrency: 0,
            requests_per_minute: 0,
            daily_limit_usd: default_zero(),
            weekly_limit_usd: default_zero(),
            max_users: None,
        }
    }
}

const fn default_true() -> bool {
    true
}

fn default_zero() -> String {
    "0".to_owned()
}

/// 校验后供认证用例使用的开通模板
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProvisionTemplate {
    pub(crate) enabled: bool,
    pub(crate) group_ids: Vec<AccountGroupId>,
    pub(crate) limits: gateway_core::policy::RateLimits,
    pub(crate) budget: ClientBudgetLimits,
    pub(crate) max_users: Option<u64>,
}

impl FeishuConfig {
    /// 校验飞书配置并编译开通模板
    ///
    /// # Errors
    ///
    /// 必填项缺失、回调地址不是绝对 HTTP(S) 地址或模板取值非法时返回错误
    pub(crate) fn validate(&self) -> Result<ProvisionTemplate, AdminConfigError> {
        let text_ok = |value: &str| {
            !value.trim().is_empty()
                && value.len() <= MAX_TEXT_BYTES
                && !value.chars().any(char::is_control)
        };
        if !text_ok(&self.app_id) {
            return Err(AdminConfigError::InvalidField("client.feishu.app_id"));
        }
        if !text_ok(self.app_secret.expose()) {
            return Err(AdminConfigError::InvalidField("client.feishu.app_secret"));
        }
        if !text_ok(&self.redirect_uri)
            || !(self.redirect_uri.starts_with("https://")
                || self.redirect_uri.starts_with("http://"))
        {
            return Err(AdminConfigError::InvalidField("client.feishu.redirect_uri"));
        }
        if self.allowed_tenant_keys.is_empty()
            || self.allowed_tenant_keys.len() > MAX_TENANTS
            || !self.allowed_tenant_keys.iter().all(|key| text_ok(key))
        {
            return Err(AdminConfigError::InvalidField(
                "client.feishu.allowed_tenant_keys",
            ));
        }
        let provision = &self.provision;
        let group_ids = provision
            .group_ids
            .iter()
            .map(|id| AccountGroupId::new(id.clone()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| AdminConfigError::InvalidField("client.feishu.provision.group_ids"))?;
        crate::model::client_keys::validate_group_ids(&group_ids)
            .map_err(|_| AdminConfigError::InvalidField("client.feishu.provision.group_ids"))?;
        if i64::try_from(provision.max_concurrency).is_err()
            || i64::try_from(provision.requests_per_minute).is_err()
        {
            return Err(AdminConfigError::InvalidField("client.feishu.provision"));
        }
        let daily_usd = provision.daily_limit_usd.parse::<Decimal>().map_err(|_| {
            AdminConfigError::InvalidField("client.feishu.provision.daily_limit_usd")
        })?;
        let weekly_usd = provision.weekly_limit_usd.parse::<Decimal>().map_err(|_| {
            AdminConfigError::InvalidField("client.feishu.provision.weekly_limit_usd")
        })?;
        if provision.max_users == Some(0) {
            return Err(AdminConfigError::InvalidField(
                "client.feishu.provision.max_users",
            ));
        }
        Ok(ProvisionTemplate {
            enabled: provision.enabled,
            group_ids,
            limits: gateway_core::policy::RateLimits {
                max_concurrency: provision.max_concurrency,
                requests_per_minute: provision.requests_per_minute,
            },
            budget: ClientBudgetLimits {
                daily_usd,
                weekly_usd,
            },
            max_users: provision.max_users,
        })
    }

    #[must_use]
    pub fn allows_tenant(&self, tenant_key: &str) -> bool {
        self.allowed_tenant_keys.iter().any(|key| key == tenant_key)
    }
}

impl fmt::Debug for FeishuConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FeishuConfig")
            .field("app_id", &self.app_id)
            .field("app_secret", &"[REDACTED]")
            .field("redirect_uri", &self.redirect_uri)
            .field("allowed_tenant_keys", &self.allowed_tenant_keys)
            .field("provision", &self.provision)
            .finish()
    }
}
