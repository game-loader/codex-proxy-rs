//! 目录对账的客户端画像，以及 Responses HTTP/SSE 的费用事实

use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroU32,
    sync::Arc,
    time::{Duration, SystemTime},
};

use futures::TryStreamExt;
use gateway_core::{
    account::{
        AccountSelectionPolicy, ProviderAccountId, RotationStrategy,
        scope::{ClientRoutingScope, FrozenAccountScope, RuntimeAccount, RuntimeAccountDirectory},
    },
    engine::{
        AccountAttemptContext, AttemptContext, ModelRequestId, RequestAttemptContext,
        provider::ProviderRequest,
    },
    event::{GatewayEvent, ProviderEvent},
    lifecycle::CancellationToken,
    metering::{ModelPriceOverride, PricingOverrides},
    operation::{GenerateRequest, Operation, OperationKind, ProtocolPayload},
    policy::ClientApiKeyId,
    routing::{
        ConfigRevision, ModelCapabilities, ProviderKind, ProviderModel, PublicModelId,
        RoutingContext, RuntimeSnapshot, UpstreamModelId,
    },
    settings::SettingsValues,
};
use serde_json::json;
use wiremock::matchers::{body_partial_json, header, method, path};
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

#[tokio::test]
async fn relay_response_stream_emits_cost_for_the_outbound_model() {
    let upstream = MockServer::start().await;
    let ports = ports_with_account("acct_relay", &format!("{}/v1", upstream.uri()));
    let provider = provider_newapi::initialize(&ports).unwrap().core_provider();
    let custom = serde_json::from_value::<ModelPriceOverride>(json!({
        "multiplierBps": 20_000,
        "bands": {"standard":{"input":"1","output":"2","cacheRead":"0.1","cacheWrite":"3"}}
    }))
    .unwrap();
    for (model, pricing, extra_usage, expected) in [
        ("glm-5.3", None, json!({}), Some("0.003258")),
        (
            "deepseek-flash",
            None,
            json!({"server_tool_use":{"tool_search":0}}),
            Some("0.0008118"),
        ),
        (
            "deepseek-flash",
            None,
            json!({"server_tool_use":{"tool_search":1}}),
            None,
        ),
        ("deepseek-flash", None, json!({"total_tokens":123}), None),
        ("glm-5.4", None, json!({}), Some("0.003258")),
        ("glm-5.4", Some(&custom), json!({}), Some("0.00386")),
    ] {
        upstream.reset().await;
        let mut usage = json!({
            "input_tokens": 1000,
            "input_tokens_details": {"cached_tokens": 300, "cache_write_tokens": 100},
            "output_tokens": 500,
            "output_tokens_details": {"reasoning_tokens": 100},
        });
        usage
            .as_object_mut()
            .unwrap()
            .extend(extra_usage.as_object().unwrap().clone());
        let created =
            json!({"type":"response.created","response":{"id":"resp_1","model":"glm-5.3"}});
        let completed = json!({
            "type":"response.completed",
            "response":{"id":"resp_1","model":"glm-5.3","status":"completed","usage":usage}
        });
        let body = format!("data: {created}\n\ndata: {completed}\n\n");
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .and(body_partial_json(json!({"model":model,"stream":true})))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
            .expect(1)
            .mount(&upstream)
            .await;

        let (request, attempt) = relay_request(model, pricing);
        let stream = Arc::clone(&provider)
            .execute(request, attempt)
            .await
            .unwrap();
        let events: Vec<ProviderEvent> = stream.try_collect().await.unwrap();
        let facts: Vec<_> = events
            .iter()
            .flat_map(ProviderEvent::canonical_facts)
            .collect();
        let usage = facts
            .iter()
            .find_map(|fact| match fact {
                GatewayEvent::Usage(usage) => Some(usage),
                _ => None,
            })
            .unwrap();
        assert_eq!(usage.cached_input_tokens, Some(300));
        assert_eq!(usage.cache_write_input_tokens, Some(100));
        let costs: Vec<_> = facts
            .iter()
            .filter_map(|fact| match fact {
                GatewayEvent::CalculatedCost(cost) => Some(cost.total().amount().canonical()),
                _ => None,
            })
            .collect();
        assert_eq!(costs, expected.into_iter().collect::<Vec<_>>(), "{model}");
        assert!(matches!(facts.last(), Some(GatewayEvent::Completed(_))));
        upstream.verify().await;
    }
}

fn relay_request(
    model: &str,
    pricing: Option<&ModelPriceOverride>,
) -> (ProviderRequest, AttemptContext) {
    let provider = ProviderKind::new("newapi").unwrap();
    let model_id = UpstreamModelId::new(model).unwrap();
    let scope = Arc::new(FrozenAccountScope::new(
        Arc::new(RuntimeAccountDirectory::new(BTreeMap::from([(
            ProviderAccountId::new("acct_relay").unwrap(),
            RuntimeAccount::new(provider.clone(), BTreeSet::new()),
        )]))),
        ClientRoutingScope::all_accounts(),
    ));
    let provider_model = ProviderModel::new(
        provider.clone(),
        model_id,
        ModelCapabilities::new(BTreeSet::from([OperationKind::Generate]), None),
    );
    let snapshot = RuntimeSnapshot::new(
        ConfigRevision::new(1).unwrap(),
        SettingsValues::new(2, "smart", Default::default(), None, None),
        vec![provider],
        vec![provider_model],
        vec![],
    )
    .unwrap();
    let payload = ProtocolPayload::json_object(
        "openai",
        json!({"model":"client-model","input":"hi"})
            .as_object()
            .unwrap()
            .clone(),
    )
    .unwrap();
    let operation = Operation::Generate(GenerateRequest::from_protocol_payload(payload));
    let plan = snapshot
        .plan(
            &PublicModelId::new(model).unwrap(),
            &operation,
            Arc::clone(&scope),
            &RoutingContext::default(),
        )
        .unwrap();
    let pricing = pricing.map_or_else(PricingOverrides::default, |pricing| {
        BTreeMap::from([(
            "newapi".to_owned(),
            BTreeMap::from([(model.to_owned(), pricing.clone())]),
        )])
    });
    let context = AttemptContext::new(
        RequestAttemptContext::new(
            ModelRequestId::new("req_relay").unwrap(),
            ClientApiKeyId::new("key_relay").unwrap(),
        )
        .with_pricing(Arc::new(pricing)),
        NonZeroU32::MIN,
        SystemTime::now() + Duration::from_secs(10),
        AccountSelectionPolicy::new(RotationStrategy::Smart, NonZeroU32::MIN),
        AccountAttemptContext::new(BTreeSet::new(), None, None).with_account_scope(scope),
        None,
        CancellationToken::new(),
    );
    (
        ProviderRequest::new(operation, plan.candidates()[0].clone()),
        context,
    )
}
