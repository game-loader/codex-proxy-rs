//! 使用 SDK 双工会话验证管理注册、分批读取与错误响应

use std::time::Duration;

use gateway_plugin_sdk::{
    CallContext, Frame, Handshake, Manifest, Message, PROTOCOL_VERSION, Stage,
    call::{
        host::KEYS_LIST,
        services::{self, usage},
    },
    client::{PluginSession, SessionConfig, read_frame, write_frame},
};
use serde_json::Value;
use tokio::io::DuplexStream;

use super::*;

async fn receive(peer: &mut DuplexStream) -> Frame {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let frame = read_frame(peer).await.unwrap();
            match frame.message {
                Message::Suspend { .. } | Message::CallbackConsumed { .. } => (),
                Message::Resume { id, epoch } => {
                    write_frame(peer, &Frame::control(Message::Resumed { id, epoch }))
                        .await
                        .unwrap();
                }
                _ => return frame,
            }
        }
    })
    .await
    .unwrap()
}

async fn call(peer: &mut DuplexStream, id: u64, method: &str, stage: Stage, params: Value) {
    write_frame(
        peer,
        &Frame {
            message: Message::Call {
                id,
                method: method.into(),
                context: CallContext {
                    request: None,
                    call_id: id,
                    instance_id: "test".into(),
                    generation: 1,
                    incarnation: "test".into(),
                    stage,
                    timeout_ms: 5_000,
                    resource_stream: false,
                    resource_scope_id: id.to_string(),
                    request_id: None,
                    attempt_id: None,
                    account_id: None,
                    credential_revision: None,
                },
                params,
            },
            payload: Vec::new(),
        },
    )
    .await
    .unwrap();
}

async fn callback(peer: &mut DuplexStream, expected_method: &str) -> (u64, Value, Vec<u8>) {
    let frame = receive(peer).await;
    let Message::Callback {
        id, method, params, ..
    } = frame.message
    else {
        panic!(
            "expected callback {expected_method}, got {:?}",
            frame.message
        );
    };
    assert_eq!(method, expected_method);
    (id, params, frame.payload)
}

async fn host_reply(peer: &mut DuplexStream, id: u64, result: Value, payload: Vec<u8>) {
    write_frame(
        peer,
        &Frame {
            message: Message::Result { id, result },
            payload,
        },
    )
    .await
    .unwrap();
}

async fn route(peer: &mut DuplexStream, id: u64, query: &str) {
    call(
        peer,
        id,
        "management.handle",
        Stage::Management,
        json!({"method": "GET", "path": "ranking", "query": query, "content_type": null}),
    )
    .await;
}

async fn result(peer: &mut DuplexStream, id: u64) -> (Value, Value) {
    let frame = receive(peer).await;
    let Message::Result {
        id: actual_id,
        result,
    } = frame.message
    else {
        panic!("expected result, got {:?}", frame.message);
    };
    assert_eq!(actual_id, id);
    (result, serde_json::from_slice(&frame.payload).unwrap())
}

async fn retention_settings(peer: &mut DuplexStream, days: u32) {
    let (id, params, payload) = callback(peer, services::CALL_METHOD).await;
    assert_eq!(params, json!({"operation":"settings.load","input":null}));
    assert!(payload.is_empty());
    let reset_weights = json!({"fiveHour":0,"weekly":0,"monthly":0});
    host_reply(peer, id, json!({"Ok": {
        "codex_privacy_policy":{"enabled":false,"onError":"skip_rule","rules":[]},
        "request_location_enabled":false,
        "request_location":{"country":"US","region":"Ohio","city":"Piketon","timezone":"America/New_York"},
        "refresh_margin_seconds":30,"refresh_concurrency":1,"max_concurrent_per_account":1,
        "max_waiting_per_key":0,"max_waiting_per_account":0,"key_queue_timeout_seconds":0,
        "account_queue_timeout_seconds":0,"switch_account_on_queue_failure":false,
        "max_retry_after_seconds":0,"openai_guardian_reserved_concurrency":0,
        "openai_account_affinity":"disabled","max_account_rotations":1,
        "openai_session_affinity_ttl_hours":1,"responses_max_decompressed_body_bytes":1024,
        "smart_scheduling":{"loadWeight":0,"quotaWeight":0,"healthWeight":0,"latencyWeight":0,
            "resetWeight":0,"queueWeight":0},
        "quota_reset_window_weights":reset_weights,
        "usage_retention_days":days,"ops_event_retention_days":31,"audit_retention_days":31,
        "account_error_protection":{"enabled":false,"rules":[],"threshold":1,"windowSeconds":1,
            "durationSeconds":1,"probeEnabled":false},
        "account_warmup_enabled":false,"account_warmup_cron":"0 0 * * *",
        "reset_credit_lead_minutes":1,"reset_credit_max_weekly_remaining_percent":0,
        "request_profiles":{},"config_revision":1,"model_mappings":{},
        "rotation_strategy":"disabled","updated_at":"2024-04-01T00:00:00Z"
    }}), Vec::new()).await;
}

fn statistics_row(
    key: Option<&str>,
    costs: &[(&str, &str)],
    tokens: Option<u64>,
) -> usage::UsageReadItem {
    let mut metrics = usage::UsageStatisticsMetrics::default();
    metrics.values.insert(
        usage::UsageMetric::RequestCount,
        usage::UsageCount(9_007_199_254_740_993),
    );
    if let Some(tokens) = tokens {
        metrics
            .values
            .insert(usage::UsageMetric::TotalTokens, usage::UsageCount(tokens));
    }
    metrics.cost_coverage.unavailable = usage::UsageCount(2);
    metrics.costs = costs
        .iter()
        .map(|&(currency, amount)| usage::UsageStatisticsCost {
            currency: currency.into(),
            amount: amount.into(),
            native_amount: None,
            contribution_amount: None,
        })
        .collect();
    usage::UsageReadItem::Statistics(Box::new(usage::UsageStatisticsRow {
        group: key.map(|key| usage::UsageStatisticsGroup {
            client_key_ref: Some(key.into()),
            ..Default::default()
        }),
        metrics,
    }))
}

fn batch(items: Vec<usage::UsageReadItem>, complete: bool) -> usage::UsageReadBatch {
    let now = Utc::now().timestamp_millis();
    usage::UsageReadBatch {
        observation: usage::UsageReadObservation {
            generated_at_ms: now,
            analytics: Some(usage::UsageReadAnalytics {
                coverage_start_ms: Some(now),
                coverage_end_ms: Some(now),
                lag_seconds: 0,
                state: "ready".into(),
                generation: 1,
            }),
            total_groups: Some(usage::UsageCount(5)),
        },
        items,
        complete,
    }
}

#[tokio::test]
async fn management_route_uses_every_statistics_batch_and_key_page() {
    let manifest = Manifest::from_author_slice(AUTHOR_MANIFEST).unwrap();
    let (mut host, plugin_peer) = tokio::io::duplex(16 * 1024);
    let server = tokio::spawn(async move {
        let (r, w) = tokio::io::split(plugin_peer);
        PluginSession::accept(r, w, SessionConfig::default())
            .await
            .unwrap()
            .run(plugin(chrono_tz::America::New_York).unwrap())
            .await
            .unwrap();
    });
    write_frame(
        &mut host,
        &Frame::control(Message::Hello {
            handshake: Handshake {
                protocol_version: PROTOCOL_VERSION,
                artifact_sha256: "a".repeat(64),
                plugin_id: manifest.plugin_id().unwrap(),
                instance_id: "test".into(),
                generation: 1,
                incarnation: "test".into(),
                configuration: json!({}),
                contributes: manifest.contributes,
            },
        }),
    )
    .await
    .unwrap();
    assert!(matches!(
        receive(&mut host).await.message,
        Message::Ready { .. }
    ));

    call(
        &mut host,
        1,
        "management.register",
        Stage::Registration,
        json!({}),
    )
    .await;
    let (_, registration) = result(&mut host, 1).await;
    assert_eq!(registration["routes"][0]["method"], "GET");
    assert_eq!(registration["routes"][0]["path"], "ranking");
    assert_eq!(registration["pages"][0]["entry"], "web/index.html");
    assert!(
        registration["resources"]
            .as_array()
            .unwrap()
            .iter()
            .all(|asset| asset["public"] == false)
    );
    assert_eq!(registration["callbacks"], json!([]));

    route(&mut host, 3, "period=month&date=2024-03-15").await;
    retention_settings(&mut host, 31).await;
    let (cb, params, _) = callback(&mut host, "host.state.get").await;
    assert_eq!(params["namespace"], "monthly-ranking");
    host_reply(&mut host, cb, json!({"record": null}), Vec::new()).await;
    let (meta, body) = result(&mut host, 3).await;
    assert_eq!(meta["status"], 422);
    assert_eq!(body["error"]["code"], "history_unavailable");

    for id in [5, 7] {
        route(&mut host, id, "period=month&date=2024-03-15").await;
        retention_settings(&mut host, 100_000).await;
        let (cb, _, payload) = callback(&mut host, usage::STATISTICS_QUERY).await;
        let query: usage::QueryUsageStatistics = serde_json::from_slice(&payload).unwrap();
        assert_eq!(query.group_by, usage::UsageGrouping::Key);
        assert_eq!(query.filter.records.start_ms, 1_709_269_200_000);
        assert_eq!(query.filter.records.end_ms, 1_711_944_000_000);
        let opened = usage::UsageReadOpened {
            reader: (id == 7).then(|| "reader".into()),
            batch: batch(
                vec![
                    statistics_row(None, &[], None),
                    statistics_row(Some("lower"), &[("USD", "9999999999.0000000001")], None),
                    statistics_row(Some("unknown"), &[("EUR", "25")], Some(0)),
                ],
                false,
            ),
        };
        host_reply(
            &mut host,
            cb,
            json!({}),
            serde_json::to_vec(&opened).unwrap(),
        )
        .await;
        if id == 5 {
            let (meta, body) = result(&mut host, id).await;
            assert_eq!(meta["status"], 502);
            assert_eq!(body["error"]["code"], "invalid_statistics");
            continue;
        }

        let (cb, _, payload) = callback(&mut host, usage::READ_NEXT).await;
        assert_eq!(
            serde_json::from_slice::<Value>(&payload).unwrap(),
            json!({"reader":"reader"})
        );
        let last = batch(
            vec![
                statistics_row(Some("highest-b"), &[("USD", "9999999999.0000000002")], None),
                statistics_row(Some("highest-a"), &[("USD", "9999999999.0000000002")], None),
                statistics_row(Some("zero"), &[("USD", "0")], None),
            ],
            true,
        );
        host_reply(&mut host, cb, json!({}), serde_json::to_vec(&last).unwrap()).await;

        let (cb, params, _) = callback(&mut host, KEYS_LIST).await;
        assert_eq!(params["cursor"], Value::Null);
        host_reply(
            &mut host,
            cb,
            json!({"keys":[],"next_cursor":"another-page"}),
            Vec::new(),
        )
        .await;
        let (cb, params, _) = callback(&mut host, KEYS_LIST).await;
        assert_eq!(params["cursor"], "another-page");
        host_reply(&mut host, cb, json!({"keys":[{"id":"highest-a","name":"Renamed key","enabled":false}],"next_cursor":null}), Vec::new()).await;

        let (meta, body) = result(&mut host, id).await;
        assert_eq!(meta["status"], 200);
        assert_eq!(body["period"], "month");
        assert_eq!(body["startDate"], "2024-03-01");
        assert_eq!(body["endDate"], "2024-03-31");
        assert_eq!(body["timezone"], "America/New_York");
        let rows = body["rows"].as_array().unwrap();
        let ids: Vec<_> = rows
            .iter()
            .map(|row| row["keyId"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["highest-a", "highest-b", "lower", "zero", "unknown"]);
        assert_eq!(
            rows[0],
            json!({"keyId":"highest-a","keyName":"Renamed key",
            "usd":"9999999999.0000000002","requests":"9007199254740993","tokens":null,"unknownCosts":"2"})
        );
        assert_eq!(rows[4]["keyName"], "unknown");
        assert_eq!(rows[4]["usd"], Value::Null);
        assert_eq!(rows[4]["tokens"], "0");
    }
    write_frame(&mut host, &Frame::control(Message::Shutdown))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
}
