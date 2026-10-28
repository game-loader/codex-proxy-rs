//! 验证外部价格源的 Provider 选择、免费价格与无效文档拒绝

use gateway_host::pricing::decode_catalog;
use serde_json::json;

#[test]
fn source_selects_supported_providers_and_preserves_free_prices() {
    let catalog = json!({
        "openai":{"models":{
            "free":{"modalities":{"output":["text"]},"cost":{"input":0,"output":0}},
            "normal":{"modalities":{"output":["text"]},"cost":{"input":2.5,"output":10,"cache_read":0.25}},
            "image":{"modalities":{"output":["image"]},"cost":{"input":1,"output":2}},
            "invalid":{"modalities":{"output":["text"]},"cost":{"input":-1,"output":2}},
            "wrong-tier":{"modalities":{"output":["text"]},"cost":{"input":1,"output":2,"tiers":[{"tier":{"size":200000},"input":2,"output":4}]}}
        }},
        "xai":{"models":{}},
        "unrelated":{"models":{"secret":{}}}
    });
    let result = decode_catalog(&serde_json::to_vec(&catalog).unwrap()).unwrap();
    let models = &result.prices["openai"];
    assert_eq!(models.len(), 2);
    assert_eq!(models["free"].bands["standard"].input.ticks_per_token(), 0);
    assert_eq!(
        models["normal"].bands["standard"]
            .cache_read
            .ticks_per_token(),
        2500
    );
    assert_eq!(result.skipped.len(), 3);
    assert!(!result.prices.contains_key("unrelated"));
}

#[test]
fn source_rejects_empty_partial_or_malformed_documents() {
    for document in [
        json!({}),
        json!({"openai":{"models":{}}}),
        json!({"openai":{"models":{}},"xai":{"models":{}}}),
    ] {
        assert!(decode_catalog(&serde_json::to_vec(&document).unwrap()).is_err());
    }
    assert!(decode_catalog(b"not json").is_err());
}

#[test]
fn relay_sources_merge_into_newapi_with_source_precedence() {
    let catalog = json!({
        "openai":{"models":{}},
        "xai":{"models":{}},
        "deepseek":{"models":{
            "deepseek-v4-flash":{"modalities":{"output":["text"]},"cost":{"input":0.15,"output":0.6,"reasoning":0.6,"cache_read":0.003}},
            "odd-reasoning":{"modalities":{"output":["text"]},"cost":{"input":1,"output":2,"reasoning":3}}
        }},
        "zai":{"models":{
            "glm-5.3":{"modalities":{"output":["text"]},"cost":{"input":1.4,"output":4.4,"cache_read":0.26,"cache_write":0}}
        }},
        "zhipuai":{"models":{
            "glm-5.3":{"modalities":{"output":["text"]},"cost":{"input":1.19,"output":4.18}},
            "glm-4.6v":{"modalities":{"output":["text"]},"cost":{"input":0.1,"output":0.4,"tiers":[{"tier":{"type":"context","size":32000},"input":0.3,"output":0.9}]}}
        }}
    });
    let result = decode_catalog(&serde_json::to_vec(&catalog).unwrap()).unwrap();
    let relayed = &result.prices["newapi"];

    assert_eq!(relayed.len(), 2);
    let flash = &relayed["deepseek-v4-flash"].bands["standard"];
    assert_eq!(flash.output.ticks_per_token(), 6_000);
    assert_eq!(flash.cache_read.ticks_per_token(), 30);
    // 同名模型采用国际站价格，不被后续来源覆盖
    assert_eq!(
        relayed["glm-5.3"].bands["standard"].input.ticks_per_token(),
        14_000
    );
    assert!(
        result
            .skipped
            .contains(&"deepseek/odd-reasoning".to_owned())
    );
    assert!(result.skipped.contains(&"zhipuai/glm-4.6v".to_owned()));
    assert!(!result.prices.contains_key("deepseek"));
}
