//! new-api 兼容上游的 API Key 凭据文档、校验与端点拼接

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// 账号认证类型；只支持 Bearer API Key
pub const AUTHENTICATION_KIND: &str = "api_key";

const SCHEMA_VERSION: u32 = 1;
const MAX_BASE_URL_CHARS: usize = 2048;
const MAX_API_KEY_BYTES: usize = 16 * 1024;

/// 持久化凭据；`base_url` 是 API 前缀，追加 `responses`、`models`
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewApiCredential {
    pub schema_version: u32,
    pub base_url: String,
    pub api_key: String,
}

impl fmt::Debug for NewApiCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NewApiCredential")
            .field("base_url", &self.base_url)
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

/// 凭据文档不合法；不回显任何字段值
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid new-api credential")]
pub struct InvalidCredential;

impl NewApiCredential {
    /// 校验并规范化导入值；去掉末尾 `/` 与完整 `/responses` 端点
    pub fn new(base_url: &str, api_key: &str) -> Result<Self, InvalidCredential> {
        let mut base_url = base_url.trim().trim_end_matches('/').to_owned();
        if let Some(prefix) = base_url.strip_suffix("/responses") {
            base_url = prefix.to_owned();
        }
        let credential = Self {
            schema_version: SCHEMA_VERSION,
            base_url,
            api_key: api_key.trim().to_owned(),
        };
        credential.validate()?;
        Ok(credential)
    }

    pub fn validate(&self) -> Result<(), InvalidCredential> {
        let base_ok = self.base_url.chars().count() <= MAX_BASE_URL_CHARS
            && !self.base_url.chars().any(char::is_control)
            && parse_base_url(&self.base_url).is_some();
        let key_ok = !self.api_key.is_empty()
            && self.api_key.len() <= MAX_API_KEY_BYTES
            && self.api_key.bytes().all(|byte| byte.is_ascii_graphic());
        if self.schema_version == SCHEMA_VERSION && base_ok && key_ok {
            Ok(())
        } else {
            Err(InvalidCredential)
        }
    }

    pub fn from_document(document: &Map<String, Value>) -> Result<Self, InvalidCredential> {
        let credential: Self = serde_json::from_value(Value::Object(document.clone()))
            .map_err(|_| InvalidCredential)?;
        credential.validate()?;
        Ok(credential)
    }

    #[must_use]
    pub fn into_document(self) -> Map<String, Value> {
        match serde_json::to_value(self) {
            Ok(Value::Object(map)) => map,
            _ => Map::new(),
        }
    }

    #[must_use]
    pub fn endpoint(&self, path: &str) -> String {
        format!("{}/{}", self.base_url.trim_end_matches('/'), path)
    }
}

// 只接受不含认证信息、查询串和 fragment 的 HTTP(S) 前缀
fn parse_base_url(value: &str) -> Option<url::Url> {
    let url = url::Url::parse(value).ok()?;
    (matches!(url.scheme(), "https" | "http")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none())
    .then_some(url)
}
