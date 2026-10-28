//! 验证飞书授权跳转的临时 Cookie、回调 state 校验与登录后的 Key 会话

use std::sync::{Arc, atomic::Ordering};

use async_trait::async_trait;
use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{Method, Request, StatusCode, header},
};
use gateway_admin::{
    FeishuAppSecret, FeishuConfig, FeishuProvisionConfig,
    model::{
        MutationContext,
        client_keys::NewClientKey,
        external_identity::{ExternalIdentity, ExternalIdentityProvider, ExternalKeyProvision},
    },
    ports::{
        external_identity::{
            ExternalIdentityExchange, ExternalIdentityStore, IdentityExchangeError,
        },
        store::AdminStoreResult,
    },
};
use gateway_core::policy::ClientApiKeyId;
use tower::ServiceExt as _;

use crate::support::{AcceptingVerifier, cookie_request, response_json};

struct Exchange {
    tenant: &'static str,
}

#[async_trait]
impl ExternalIdentityExchange for Exchange {
    fn authorize_url(&self, state: &str, code_challenge: &str) -> String {
        format!(
            "https://accounts.feishu.test/authorize?state={state}&code_challenge={code_challenge}"
        )
    }

    async fn exchange(
        &self,
        code: &str,
        _: &str,
    ) -> Result<ExternalIdentity, IdentityExchangeError> {
        if code != "good-code" {
            return Err(IdentityExchangeError::Rejected);
        }
        Ok(ExternalIdentity {
            provider: ExternalIdentityProvider::Feishu,
            tenant_key: self.tenant.to_owned(),
            subject: "on_member".to_owned(),
            display_name: None,
        })
    }
}

// 绑定指向夹具中唯一启用的 Key，复用会话存储的启用检查
struct BoundToFixtureKey;

#[async_trait]
impl ExternalIdentityStore for BoundToFixtureKey {
    async fn find_bound_key(
        &self,
        _: ExternalIdentityProvider,
        _: &str,
        _: &str,
    ) -> AdminStoreResult<Option<ClientApiKeyId>> {
        Ok(Some(ClientApiKeyId::new("key-42").unwrap()))
    }

    async fn provision_key(
        &self,
        _: &ExternalIdentity,
        _: NewClientKey,
        _: Option<u64>,
        _: &MutationContext,
    ) -> AdminStoreResult<ExternalKeyProvision> {
        unreachable!("bound identity must not provision")
    }
}

fn config(redirect_uri: &str) -> FeishuConfig {
    FeishuConfig {
        app_id: "cli_test".to_owned(),
        app_secret: FeishuAppSecret::new("secret"),
        redirect_uri: redirect_uri.to_owned(),
        allowed_tenant_keys: vec!["tenant-a".to_owned()],
        provision: FeishuProvisionConfig::default(),
    }
}

async fn app(tenant: &'static str, redirect_uri: &str) -> axum::Router {
    let fixture = crate::admin::AdminTestFixture::with_feishu(
        Arc::new(AcceptingVerifier {
            key_id: ClientApiKeyId::new("key-42").unwrap(),
        }),
        (
            config(redirect_uri),
            Arc::new(Exchange { tenant }),
            Arc::new(BoundToFixtureKey),
        ),
    )
    .await;
    fixture.auth.enabled.store(true, Ordering::SeqCst);
    crate::openai::api_router_with_admin(fixture.services)
}

fn set_cookies(response: &axum::response::Response) -> Vec<String> {
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|value| value.to_str().unwrap().to_owned())
        .collect()
}

fn location(response: &axum::response::Response) -> &str {
    response.headers()[header::LOCATION].to_str().unwrap()
}

async fn authorize(app: &axum::Router) -> (String, String) {
    let response = app
        .clone()
        .oneshot(
            Request::get("/api/auth/feishu/authorize")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let flow = set_cookies(&response)
        .into_iter()
        .find(|cookie| cookie.starts_with("cpr_feishu_flow="))
        .expect("flow cookie");
    let pair = flow.split(';').next().unwrap().to_owned();
    let state = pair
        .trim_start_matches("cpr_feishu_flow=")
        .split('.')
        .next()
        .unwrap()
        .to_owned();
    assert!(location(&response).contains(&format!("state={state}")));
    (pair, state)
}

async fn callback(app: &axum::Router, cookie: &str, query: &str) -> axum::response::Response {
    let mut request = Request::builder()
        .method(Method::GET)
        .uri(format!("/api/auth/feishu/callback?{query}"))
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo(std::net::SocketAddr::from((
            [203, 0, 113, 20],
            41_000,
        ))));
    app.clone().oneshot(request).await.unwrap()
}

#[tokio::test]
async fn authorize_sets_scoped_flow_cookie_and_callback_issues_key_session() {
    let app = app(
        "tenant-a",
        "https://proxy.example.test/api/auth/feishu/callback",
    )
    .await;
    let (flow, state) = authorize(&app).await;

    let response = callback(&app, &flow, &format!("code=good-code&state={state}")).await;

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&response), "/key-usage");
    let cookies = set_cookies(&response);
    let session = cookies
        .iter()
        .find(|cookie| cookie.starts_with("cpr_session=session_"))
        .expect("session cookie");
    assert!(session.contains("Secure; HttpOnly; SameSite=Lax"));
    assert!(cookies.iter().any(|cookie| {
        cookie.starts_with("cpr_feishu_flow=;")
            && cookie.contains("Path=/api/auth/feishu")
            && cookie.contains("Max-Age=0")
    }));
    let status = app
        .clone()
        .oneshot(cookie_request(
            Method::GET,
            "/api/auth/status",
            session.split(';').next().unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(
        response_json(status).await["data"]["session"]["role"],
        "key"
    );
}

#[tokio::test]
async fn callback_rejects_mismatched_state_missing_flow_and_foreign_tenants() {
    let app_a = app("tenant-a", "http://127.0.0.1:8080/api/auth/feishu/callback").await;
    let (flow, state) = authorize(&app_a).await;
    assert!(!flow.contains("Secure"));

    for (cookie, query) in [
        (flow.as_str(), "code=good-code&state=forged".to_owned()),
        ("", format!("code=good-code&state={state}")),
        (flow.as_str(), format!("state={state}")),
    ] {
        let response = callback(&app_a, cookie, &query).await;
        assert_eq!(location(&response), "/login?feishu_error=denied");
        assert!(
            !set_cookies(&response)
                .iter()
                .any(|cookie| cookie.starts_with("cpr_session=session_"))
        );
    }
    let rejected = callback(&app_a, &flow, &format!("code=bad-code&state={state}")).await;
    assert_eq!(location(&rejected), "/login?feishu_error=denied");

    let foreign = app("tenant-b", "http://127.0.0.1:8080/api/auth/feishu/callback").await;
    let (flow, state) = authorize(&foreign).await;
    let response = callback(&foreign, &flow, &format!("code=good-code&state={state}")).await;
    assert_eq!(location(&response), "/login?feishu_error=forbidden");
}

#[tokio::test]
async fn login_options_report_whether_feishu_is_configured() {
    let configured = app("tenant-a", "https://proxy.example.test/cb").await;
    let response = configured
        .oneshot(
            Request::get("/api/auth/options")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response_json(response).await["data"]["feishu"], true);

    let (plain, _) = crate::support::auth_app().await;
    let response = plain
        .clone()
        .oneshot(
            Request::get("/api/auth/options")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response_json(response).await["data"]["feishu"], false);
    let response = plain
        .oneshot(
            Request::get("/api/auth/feishu/authorize")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(location(&response), "/login?feishu_error=unavailable");
}
