//! 按账号出站代理缓存的 reqwest 客户端与上游 HTTP 错误分类

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use gateway_core::account::OutboundProxy;
use gateway_core::error::{
    ClientVisibleUpstreamError, ErrorSource, OpaqueUpstreamValue, ProviderError, ProviderErrorKind,
};
use gateway_core::upstream::UpstreamSendState;
use serde_json::Value;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_CACHED_CLIENTS: usize = 64;
/// 错误响应只读取有限正文用于分类和展示
pub(crate) const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
#[error("new-api HTTP client could not be built")]
pub(crate) struct ClientBuildError;

/// 直连与每个代理地址各复用一个连接池；客户端本身不设总超时，由调用方按期限控制
pub(crate) fn client(proxy: Option<&OutboundProxy>) -> Result<reqwest::Client, ClientBuildError> {
    static CLIENTS: OnceLock<Mutex<HashMap<String, reqwest::Client>>> = OnceLock::new();
    let key = proxy.map_or_else(String::new, |proxy| proxy.expose_url().to_owned());
    let cache = CLIENTS.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(client) = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&key)
    {
        return Ok(client.clone());
    }
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(CONNECT_TIMEOUT);
    if let Some(proxy) = proxy {
        builder =
            builder.proxy(reqwest::Proxy::all(proxy.expose_url()).map_err(|_| ClientBuildError)?);
    }
    let client = builder.build().map_err(|_| ClientBuildError)?;
    let mut clients = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if clients.len() >= MAX_CACHED_CLIENTS {
        clients.clear();
    }
    Ok(clients.entry(key).or_insert(client).clone())
}

/// 发送阶段失败；连接未建立时可确认未发送，其余结果无法证明上游未处理
pub(crate) fn send_error(error: reqwest::Error) -> ProviderError {
    let (kind, state) = if error.is_connect() {
        (ProviderErrorKind::Transport, UpstreamSendState::NotSent)
    } else if error.is_timeout() {
        (ProviderErrorKind::Timeout, UpstreamSendState::Ambiguous)
    } else {
        (ProviderErrorKind::Transport, UpstreamSendState::Ambiguous)
    };
    ProviderError::new(kind, state).with_source(ErrorSource::new(error))
}

/// 读取响应流中途失败；请求已经发出
pub(crate) fn stream_error(error: reqwest::Error) -> ProviderError {
    let kind = if error.is_timeout() {
        ProviderErrorKind::Timeout
    } else {
        ProviderErrorKind::Transport
    };
    ProviderError::new(kind, UpstreamSendState::Sent).with_source(ErrorSource::new(error))
}

/// 非 2xx 响应按状态码分类，保留结构化上游文案供客户端展示
pub(crate) fn status_error(
    status: reqwest::StatusCode,
    retry_after: Option<Duration>,
    body: &[u8],
) -> ProviderError {
    let kind = match status.as_u16() {
        400 | 404 | 413 | 422 => ProviderErrorKind::InvalidRequest,
        401 => ProviderErrorKind::Unauthorized,
        402 => ProviderErrorKind::QuotaExhausted,
        403 => ProviderErrorKind::PermissionDenied,
        408 => ProviderErrorKind::Timeout,
        429 => ProviderErrorKind::RateLimited,
        _ => ProviderErrorKind::Unavailable,
    };
    let mut error = ProviderError::new(kind, UpstreamSendState::Sent).with_status(status.as_u16());
    if let Some(retry_after) = retry_after {
        error = error.with_retry_after(retry_after);
    }
    let parsed = serde_json::from_slice::<Value>(body).ok();
    let field = |name: &str| {
        parsed.as_ref().and_then(|value| {
            value
                .pointer(&format!("/error/{name}"))
                .or_else(|| value.get(name))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
    };
    let code = field("code");
    if let Some(code) = code.clone() {
        error = error.with_upstream_code(OpaqueUpstreamValue::new(code));
    }
    if let Some(message) = field("message") {
        error = error.with_client_visible_upstream_error(ClientVisibleUpstreamError::new(
            message,
            code,
            field("type"),
        ));
    }
    error
}

pub(crate) fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(gateway_protocol::openai::parse_retry_after_seconds)
        .map(Duration::from_secs)
}

/// 读取有界正文；超过上限时截断，不影响状态码分类
pub(crate) async fn bounded_body(mut response: reqwest::Response, limit: usize) -> Vec<u8> {
    let mut body = Vec::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        let remaining = limit.saturating_sub(body.len());
        if remaining == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }
    body
}
