//! 飞书网页应用授权码登录的出站适配；固定官方端点、禁止跳转，不进入推理请求链路

use std::time::Duration;

use async_trait::async_trait;
use gateway_admin::{
    FeishuConfig,
    model::external_identity::{ExternalIdentity, ExternalIdentityProvider},
    ports::external_identity::{ExternalIdentityExchange, IdentityExchangeError},
};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;

const AUTHORIZE_URL: &str = "https://accounts.feishu.cn/open-apis/authen/v1/authorize";
const TOKEN_URL: &str = "https://open.feishu.cn/open-apis/authen/v2/oauth/token";
const USER_INFO_URL: &str = "https://open.feishu.cn/open-apis/authen/v1/user_info";
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_IDENTIFIER_BYTES: usize = 128;

pub struct FeishuIdentityExchange {
    client: reqwest::Client,
    app_id: String,
    app_secret: String,
    redirect_uri: String,
}

impl FeishuIdentityExchange {
    /// # Errors
    ///
    /// HTTP 客户端无法初始化时返回错误
    pub fn new(config: &FeishuConfig) -> Result<Self, reqwest::Error> {
        let client = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(15))
            .build()?;
        Ok(Self {
            client,
            app_id: config.app_id.clone(),
            app_secret: config.app_secret.expose().to_owned(),
            redirect_uri: config.redirect_uri.clone(),
        })
    }

    async fn read_json<T: for<'de> Deserialize<'de>>(
        mut response: reqwest::Response,
    ) -> Result<T, IdentityExchangeError> {
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| IdentityExchangeError::Unavailable)?
        {
            if chunk.len() > MAX_RESPONSE_BYTES.saturating_sub(body.len()) {
                return Err(IdentityExchangeError::Unavailable);
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body).map_err(|_| IdentityExchangeError::Unavailable)
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    code: i64,
    access_token: Option<String>,
}

#[derive(Deserialize)]
struct UserInfoResponse {
    code: i64,
    data: Option<UserInfo>,
}

#[derive(Deserialize)]
struct UserInfo {
    union_id: Option<String>,
    tenant_key: Option<String>,
    name: Option<String>,
}

// 20050 与 20072 为可重试的服务端故障，其余非零码按授权失败处理
fn token_error(code: i64) -> IdentityExchangeError {
    match code {
        20050 | 20072 => IdentityExchangeError::Unavailable,
        _ => IdentityExchangeError::Rejected,
    }
}

fn identifier(value: Option<String>) -> Result<String, IdentityExchangeError> {
    value
        .filter(|value| {
            !value.is_empty()
                && value.len() <= MAX_IDENTIFIER_BYTES
                && value.bytes().all(|byte| byte.is_ascii_graphic())
        })
        .ok_or(IdentityExchangeError::Unavailable)
}

#[async_trait]
impl ExternalIdentityExchange for FeishuIdentityExchange {
    fn authorize_url(&self, state: &str, code_challenge: &str) -> String {
        let encode = |value: &str| utf8_percent_encode(value, NON_ALPHANUMERIC).to_string();
        format!(
            "{AUTHORIZE_URL}?client_id={}&response_type=code&redirect_uri={}&state={}&code_challenge={}&code_challenge_method=S256",
            encode(&self.app_id),
            encode(&self.redirect_uri),
            encode(state),
            encode(code_challenge),
        )
    }

    async fn exchange(
        &self,
        code: &str,
        code_verifier: &str,
    ) -> Result<ExternalIdentity, IdentityExchangeError> {
        let response = self
            .client
            .post(TOKEN_URL)
            .json(&serde_json::json!({
                "grant_type": "authorization_code",
                "client_id": self.app_id,
                "client_secret": self.app_secret,
                "code": code,
                "redirect_uri": self.redirect_uri,
                "code_verifier": code_verifier,
            }))
            .send()
            .await
            .map_err(|_| IdentityExchangeError::Unavailable)?;
        let status = response.status();
        // 失败响应同样携带 code，状态码只用于识别无法解析时的服务端故障
        let token: TokenResponse = match Self::read_json(response).await {
            Ok(token) => token,
            Err(_) if status.is_client_error() => return Err(IdentityExchangeError::Rejected),
            Err(error) => return Err(error),
        };
        if token.code != 0 {
            return Err(token_error(token.code));
        }
        let access_token = token
            .access_token
            .filter(|token| !token.is_empty())
            .ok_or(IdentityExchangeError::Unavailable)?;
        let response = self
            .client
            .get(USER_INFO_URL)
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|_| IdentityExchangeError::Unavailable)?;
        let info: UserInfoResponse = Self::read_json(response).await?;
        if info.code != 0 {
            // 20021 离职、20022 冻结、20008 不存在等都不允许登录
            return Err(match info.code {
                20050 => IdentityExchangeError::Unavailable,
                _ => IdentityExchangeError::Rejected,
            });
        }
        let data = info.data.ok_or(IdentityExchangeError::Unavailable)?;
        Ok(ExternalIdentity {
            provider: ExternalIdentityProvider::Feishu,
            tenant_key: identifier(data.tenant_key)?,
            subject: identifier(data.union_id)?,
            display_name: data.name.filter(|name| !name.trim().is_empty()),
        })
    }
}
