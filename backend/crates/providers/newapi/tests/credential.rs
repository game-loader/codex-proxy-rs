//! 凭据规范化与校验边界

use provider_newapi::{NewApiCredential, parse_models};

#[test]
fn normalizes_base_url_and_builds_endpoints() {
    let credential =
        NewApiCredential::new(" https://relay.example.com/v1/responses/ ", " sk-test ").unwrap();
    assert_eq!(credential.base_url, "https://relay.example.com/v1");
    assert_eq!(credential.api_key, "sk-test");
    assert_eq!(
        credential.endpoint("responses"),
        "https://relay.example.com/v1/responses"
    );
    // 调试输出不能泄露密钥
    assert!(!format!("{credential:?}").contains("sk-test"));
}

#[test]
fn rejects_unsafe_or_incomplete_credentials() {
    for (base_url, api_key) in [
        ("ftp://relay.example.com/v1", "sk"),
        ("https://user:pass@relay.example.com/v1", "sk"),
        ("https://relay.example.com/v1?x=1", "sk"),
        ("https://relay.example.com/v1#frag", "sk"),
        ("https://relay.example.com/v1", ""),
        ("https://relay.example.com/v1", "sk with space"),
    ] {
        assert!(
            NewApiCredential::new(base_url, api_key).is_err(),
            "{base_url} {api_key}"
        );
    }
}

#[test]
fn document_round_trip_rejects_unknown_fields() {
    let credential = NewApiCredential::new("https://relay.example.com/v1", "sk-test").unwrap();
    let mut document = credential.clone().into_document();
    assert_eq!(
        NewApiCredential::from_document(&document).unwrap(),
        credential
    );
    document.insert("extra".to_owned(), serde_json::json!(true));
    assert!(NewApiCredential::from_document(&document).is_err());
}

#[test]
fn model_list_is_sorted_deduplicated_and_filtered() {
    let body = br#"{"data":[{"id":"glm-5.3"},{"id":"deepseek-v4-flash"},{"id":"glm-5.3"},{"id":"bad id"},{"id":""},{"object":"model"}]}"#;
    assert_eq!(
        parse_models(body).unwrap(),
        ["deepseek-v4-flash", "glm-5.3"]
    );
    assert!(parse_models(b"{}").is_err());
    assert!(parse_models(b"not json").is_err());
}
