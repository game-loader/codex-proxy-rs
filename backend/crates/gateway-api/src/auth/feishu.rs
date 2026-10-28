//! 飞书授权跳转与回调；state 与 PKCE verifier 暂存在仅回调路径可见的 HttpOnly Cookie

use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use gateway_admin::model::auth::{LoginCommand, LoginError};
use serde::Deserialize;

use super::{SessionState, set_session_cookie_with};
use crate::session_cookie;

const FLOW_COOKIE: &str = "cpr_feishu_flow";
const FLOW_PATH: &str = "/api/auth/feishu";
const FLOW_TTL_SECONDS: u32 = 600;
const LOGIN_PAGE: &str = "/login";
const KEY_USAGE_PAGE: &str = "/key-usage";

#[derive(Deserialize)]
pub(super) struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
}

pub(super) async fn authorize<S>(State(state): State<S>) -> Response
where
    S: SessionState + Send + Sync,
{
    let Some(authorization) = state.admin_services().auth().feishu_authorization() else {
        return login_redirect("unavailable");
    };
    let mut response = redirect(&authorization.url);
    let cookie = format!(
        "{FLOW_COOKIE}={}.{}; Path={FLOW_PATH}; HttpOnly; SameSite=Lax; Max-Age={FLOW_TTL_SECONDS}{}",
        authorization.state,
        authorization.code_verifier,
        if authorization.secure_callback {
            "; Secure"
        } else {
            ""
        },
    );
    let Ok(cookie) = HeaderValue::from_str(&cookie) else {
        return login_redirect("unavailable");
    };
    response.headers_mut().append(header::SET_COOKIE, cookie);
    response
}

pub(super) async fn callback<S>(
    State(state): State<S>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<CallbackQuery>,
) -> Response
where
    S: SessionState + Send + Sync,
{
    let auth = state.admin_services().auth();
    let Some(secure) = auth.feishu_secure_callback() else {
        return login_redirect("unavailable");
    };
    let flow = flow_cookie(&headers);
    // 一次授权只能回调一次，无论结果都清除暂存的 state 与 verifier
    let finish = |mut response: Response| {
        let clear = format!(
            "{FLOW_COOKIE}=; Path={FLOW_PATH}; HttpOnly; SameSite=Lax; Max-Age=0{}",
            if secure { "; Secure" } else { "" },
        );
        if let Ok(value) = HeaderValue::from_str(&clear) {
            response.headers_mut().append(header::SET_COOKIE, value);
        }
        response
    };
    let (Some((expected_state, code_verifier)), Some(code), Some(returned_state)) =
        (flow, query.code, query.state)
    else {
        return finish(login_redirect("denied"));
    };
    if expected_state != returned_state {
        return finish(login_redirect("denied"));
    }
    let result = auth
        .login(
            LoginCommand::Feishu {
                code,
                code_verifier,
            },
            peer.ip(),
            session_cookie::refresh_value(&headers)
                .or_else(|| session_cookie::value(&headers))
                .as_deref(),
        )
        .await;
    let grant = match result {
        Ok(grant) => grant,
        Err(error) => return finish(login_redirect(error_reason(error))),
    };
    // 回调是顶层导航，没有 Origin；会话 Cookie 的 Secure 属性按登记的回调协议决定
    let attributes = if secure {
        "Secure; HttpOnly; SameSite=Lax"
    } else {
        "HttpOnly; SameSite=Lax"
    };
    match set_session_cookie_with(redirect(KEY_USAGE_PAGE), attributes, &grant) {
        Ok(response) => finish(response),
        Err(_) => finish(login_redirect("unavailable")),
    }
}

fn flow_cookie(headers: &HeaderMap) -> Option<(String, String)> {
    let value = headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .find_map(|part| {
            let (name, value) = part.trim().split_once('=')?;
            (name == FLOW_COOKIE).then(|| value.to_owned())
        })?;
    let (state, verifier) = value.split_once('.')?;
    (!state.is_empty() && !verifier.is_empty()).then(|| (state.to_owned(), verifier.to_owned()))
}

const fn error_reason(error: LoginError) -> &'static str {
    match error {
        LoginError::AccessDenied => "forbidden",
        LoginError::TooManyAttempts { .. } => "rate_limited",
        LoginError::Unavailable => "unavailable",
        LoginError::InvalidCredentials => "denied",
    }
}

fn login_redirect(reason: &str) -> Response {
    redirect(&format!("{LOGIN_PAGE}?feishu_error={reason}"))
}

fn redirect(location: &str) -> Response {
    let mut response = StatusCode::SEE_OTHER.into_response();
    if let Ok(value) = HeaderValue::from_str(location) {
        response.headers_mut().insert(header::LOCATION, value);
    }
    response
}
