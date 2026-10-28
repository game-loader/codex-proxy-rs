//! 验证飞书授权地址固定官方端点并对查询参数完整编码

use gateway_admin::{
    FeishuAppSecret, FeishuConfig, FeishuProvisionConfig,
    ports::external_identity::ExternalIdentityExchange as _,
};
use gateway_host::feishu::FeishuIdentityExchange;

#[test]
fn authorize_url_targets_official_endpoint_with_encoded_pkce_parameters() {
    let exchange = FeishuIdentityExchange::new(&FeishuConfig {
        app_id: "cli_a1b2".to_owned(),
        app_secret: FeishuAppSecret::new("secret-value"),
        redirect_uri: "https://proxy.example.com/api/auth/feishu/callback?x=1".to_owned(),
        allowed_tenant_keys: vec!["tenant".to_owned()],
        provision: FeishuProvisionConfig::default(),
    })
    .expect("client");

    let url = exchange.authorize_url("st-1", "ch_2");

    assert!(url.starts_with("https://accounts.feishu.cn/open-apis/authen/v1/authorize?"));
    assert!(url.contains("client_id=cli%5Fa1b2"));
    assert!(url.contains("response_type=code"));
    assert!(url.contains(
        "redirect_uri=https%3A%2F%2Fproxy%2Eexample%2Ecom%2Fapi%2Fauth%2Ffeishu%2Fcallback%3Fx%3D1"
    ));
    assert!(url.contains("state=st%2D1"));
    assert!(url.contains("code_challenge=ch%5F2&code_challenge_method=S256"));
    assert!(!url.contains("secret-value"));
}
