//! 目录对账产出的模型能力与 Codex 客户端画像

use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::support::ports_with_account;

// Codex 客户端目录只列出带画像的模型，缺少画像会让 `/model` 中看不到这些模型
#[tokio::test]
async fn discovered_models_carry_a_generic_codex_presentation() {
    let upstream = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("authorization", "Bearer sk-test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{ "id": "kimi-k3.1" }, { "id": "glm-5.2" }],
        })))
        .expect(1)
        .mount(&upstream)
        .await;
    let ports = ports_with_account("acct_relay", &format!("{}/v1", upstream.uri()));
    let provider = provider_newapi::initialize(&ports).unwrap().core_provider();

    let models = provider.query_model_capabilities().await.unwrap();

    let ids: Vec<_> = models
        .iter()
        .map(|model| model.upstream_model().as_str())
        .collect();
    assert_eq!(ids, ["glm-5.2", "kimi-k3.1"]);
    for model in &models {
        let presentation = model.presentation().expect("每个上游模型都要带 Codex 画像");
        assert_eq!(
            presentation.display_name(),
            Some(model.upstream_model().as_str())
        );
        // 上游只给 ID，推理档位与上下文窗口保持未知，只开启 Responses 工具
        assert!(presentation.agent_tools());
        assert!(!presentation.parallel_tool_calls());
        assert!(presentation.supported_reasoning_efforts().is_empty());
        assert_eq!(presentation.context_window_tokens(), None);
    }
}
