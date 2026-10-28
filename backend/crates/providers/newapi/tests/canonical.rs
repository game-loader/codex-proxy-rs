//! Responses SSE 解码：原字节透传、用量计价与终态边界

use std::collections::BTreeMap;

use gateway_core::event::{GatewayEvent, ProviderEvent};
use gateway_core::metering::{ModelPriceOverride, TokenPrice, TokenPriceOverride};
use provider_newapi::NewApiCanonicalDecoder;

fn price(value: &str) -> TokenPrice {
    TokenPrice::try_from(value.to_owned()).unwrap()
}

fn pricing() -> ModelPriceOverride {
    ModelPriceOverride {
        multiplier_bps: 10_000,
        bands: BTreeMap::from([(
            "standard".to_owned(),
            TokenPriceOverride {
                input: price("1"),
                output: price("2"),
                cache_read: price("0.1"),
                cache_write: price("1"),
            },
        )]),
    }
}

fn frame(value: serde_json::Value) -> String {
    format!(
        "event: {}\ndata: {value}\n\n",
        value["type"].as_str().unwrap()
    )
}

fn stream() -> String {
    [
        frame(serde_json::json!({"type":"response.created","response":{"id":"resp_1","model":"glm-5.3","status":"in_progress"}})),
        frame(serde_json::json!({"type":"response.output_item.added","output_index":0,"item":{"type":"message","id":"msg_1","role":"assistant","content":[]}})),
        frame(serde_json::json!({"type":"response.content_part.added","output_index":0,"content_index":0,"item_id":"msg_1","part":{"type":"output_text","text":""}})),
        frame(serde_json::json!({"type":"response.output_text.delta","output_index":0,"content_index":0,"item_id":"msg_1","delta":"hi"})),
        ": ping\n\n".to_owned(),
        frame(serde_json::json!({"type":"response.completed","response":{"id":"resp_1","model":"glm-5.3","status":"completed","usage":{"input_tokens":1000,"input_tokens_details":{"cached_tokens":200},"output_tokens":500,"output_tokens_details":{"reasoning_tokens":100},"total_tokens":1500}}})),
    ]
    .concat()
}

fn facts(events: &[ProviderEvent]) -> Vec<&GatewayEvent> {
    events
        .iter()
        .flat_map(ProviderEvent::canonical_facts)
        .collect()
}

#[test]
fn passes_bytes_through_and_prices_usage() {
    let body = stream();
    let mut decoder = NewApiCanonicalDecoder::new("glm-5.3", Some(pricing()));
    // 逐字节推入，验证跨块帧仍按原字节交付
    let mut events = Vec::new();
    for byte in body.as_bytes() {
        events.extend(decoder.push(std::slice::from_ref(byte)).unwrap());
    }
    events.extend(decoder.finish().unwrap());
    assert!(decoder.is_completed());

    let raw: Vec<u8> = events
        .iter()
        .filter_map(|event| event.wire_event()?.raw_sse_frame().cloned())
        .flat_map(|bytes| bytes.to_vec())
        .collect();
    assert_eq!(raw, body.as_bytes());
    assert!(events.iter().all(|event| {
        event
            .wire_event()
            .is_some_and(|wire| wire.protocol() == "openai")
    }));

    let facts = facts(&events);
    assert!(matches!(facts.first(), Some(GatewayEvent::Started(_))));
    assert!(matches!(facts.last(), Some(GatewayEvent::Completed(_))));
    // 800 未缓存 × 1 + 200 缓存 × 0.1 + 500 输出 × 2，单位 USD / 百万 Token
    let cost = facts
        .iter()
        .find_map(|event| match event {
            GatewayEvent::CalculatedCost(cost) => Some(cost.total().amount().scaled()),
            _ => None,
        })
        .expect("calculated cost");
    let expected = provider_newapi::billing_breakdown(1000, 500, 200, 0, Some(&pricing()))
        .unwrap()
        .total_amount()
        .amount()
        .scaled();
    assert_eq!(cost, expected);
    assert!(cost > 0);
}

#[test]
fn skips_cost_without_pricing() {
    let mut decoder = NewApiCanonicalDecoder::new("glm-5.3", None);
    let mut events = decoder.push(stream().as_bytes()).unwrap();
    events.extend(decoder.finish().unwrap());
    let facts = facts(&events);
    assert!(
        facts
            .iter()
            .any(|event| matches!(event, GatewayEvent::Usage(_)))
    );
    assert!(
        !facts
            .iter()
            .any(|event| matches!(event, GatewayEvent::CalculatedCost(_)))
    );
}

#[test]
fn failed_event_and_missing_terminal_are_errors() {
    let mut decoder = NewApiCanonicalDecoder::new("glm-5.3", None);
    let failed = [
        frame(serde_json::json!({"type":"response.created","response":{"id":"resp_1","status":"in_progress"}})),
        frame(serde_json::json!({"type":"response.failed","response":{"id":"resp_1","status":"failed","error":{"code":"server_error","message":"boom"}}})),
    ]
    .concat();
    assert!(decoder.push(failed.as_bytes()).is_err());

    let mut decoder = NewApiCanonicalDecoder::new("glm-5.3", None);
    let truncated = frame(
        serde_json::json!({"type":"response.created","response":{"id":"resp_1","status":"in_progress"}}),
    );
    decoder.push(truncated.as_bytes()).unwrap();
    assert!(decoder.finish().is_err());
}
