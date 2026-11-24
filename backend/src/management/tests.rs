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

async fn start(timezone: Tz) -> (DuplexStream, tokio::task::JoinHandle<()>) {
    let manifest = Manifest::from_author_slice(AUTHOR_MANIFEST).unwrap();
    let (mut host, plugin_peer) = tokio::io::duplex(16 * 1024);
    let server = tokio::spawn(async move {
        let (r, w) = tokio::io::split(plugin_peer);
        PluginSession::accept(r, w, SessionConfig::default())
            .await
            .unwrap()
            .run(plugin(timezone).unwrap())
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

    (host, server)
}

async fn stop(mut host: DuplexStream, server: tokio::task::JoinHandle<()>) {
    write_frame(&mut host, &Frame::control(Message::Shutdown))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn management_route_uses_every_statistics_batch_and_key_page() {
    let (mut host, server) = start(chrono_tz::America::New_York).await;
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
    stop(host, server).await;
}

async fn snapshot(host: &mut DuplexStream, namespace: &str, day: &Window, rows: Option<Value>) {
    let (id, params, _) = callback(host, "host.state.get").await;
    assert_eq!(params["namespace"], namespace);
    assert_eq!(params["key"], format!("UTC/{}", day.start_date));
    let record = rows.map(|rows| {
        json!({
            "version": 1, "schema_version": 1,
            "value": {"start_ms": day.start_ms, "end_ms": day.end_ms, "rows": rows}
        })
    });
    host_reply(host, id, json!({"record": record}), Vec::new()).await;
}

#[tokio::test]
async fn legacy_archive_preserves_key_totals_without_fabricating_model_rows() {
    let timezone = chrono_tz::UTC;
    let (mut host, server) = start(timezone).await;
    let february = Window::parse("period=month&date=2024-02-01", timezone, Utc::now())
        .unwrap()
        .0;
    let day = Window::day(february.start_date, timezone).unwrap();
    let totals = json!({"stable": ["4.25", "2", "20", "1"]});

    for (id, key, expected_status) in [(1, "stable", 422), (3, "other", 200)] {
        route(
            &mut host,
            id,
            &format!("period=day&date=2024-02-01&keyId={key}"),
        )
        .await;
        retention_settings(&mut host, 31).await;
        snapshot(&mut host, "model-ranking", &day, None).await;
        snapshot(&mut host, "monthly-ranking", &day, Some(totals.clone())).await;
        let (meta, body) = result(&mut host, id).await;
        assert_eq!(meta["status"], expected_status);
        if expected_status == 422 {
            assert_eq!(body["error"]["code"], "history_unavailable");
        } else {
            assert_eq!(body["models"], json!([]));
        }
    }

    route(&mut host, 5, "period=month&date=2024-02-01").await;
    retention_settings(&mut host, 31).await;
    for date in february.start_date.iter_days().take(29) {
        let day = Window::day(date, timezone).unwrap();
        let rows = if date == february.start_date {
            totals.clone()
        } else {
            json!({})
        };
        snapshot(&mut host, "monthly-ranking", &day, Some(rows)).await;
    }
    let (cb, _, _) = callback(&mut host, KEYS_LIST).await;
    host_reply(
        &mut host,
        cb,
        json!({"keys":[],"next_cursor":null}),
        Vec::new(),
    )
    .await;
    let (meta, body) = result(&mut host, 5).await;
    assert_eq!(meta["status"], 200);
    assert_eq!(
        body["rows"],
        json!([{
            "keyId":"stable", "keyName":"stable", "usd":"4.25",
            "requests":"2", "tokens":"20", "unknownCosts":"1"
        }])
    );
    stop(host, server).await;
}

fn model_statistics_row(
    model: Option<&str>,
    unassigned_contribution: bool,
    usd: Option<&str>,
    tokens: Option<u64>,
) -> usage::UsageReadItem {
    let costs: Vec<_> = usd.map(|amount| ("USD", amount)).into_iter().collect();
    let mut item = statistics_row(Some("stable"), &costs, tokens);
    let usage::UsageReadItem::Statistics(row) = &mut item else {
        unreachable!();
    };
    let group = row.group.as_mut().unwrap();
    group.model = model.map(str::to_owned);
    group.unassigned_contribution = unassigned_contribution;
    item
}

#[tokio::test]
async fn model_detail_merges_archived_and_filtered_live_groups_with_exact_ordering() {
    let timezone = chrono_tz::UTC;
    let (mut host, server) = start(timezone).await;
    let now = Utc::now();
    let previous_week = now.date_naive() - chrono::Days::new(7);
    let query = format!("period=week&date={previous_week}&keyId=stable");
    let week = Window::parse(&query, timezone, now).unwrap().0;
    let days = (now.date_naive() - week.start_date).num_days() as u32 - 2;
    let first_live = week
        .start_date
        .iter_days()
        .find(|date| {
            Window::day(*date, timezone)
                .unwrap()
                .is_retained_at(now.timestamp_millis(), days)
        })
        .unwrap();

    route(&mut host, 1, &query).await;
    retention_settings(&mut host, days).await;
    for date in week
        .start_date
        .iter_days()
        .take_while(|date| *date < first_live)
    {
        let day = Window::day(date, timezone).unwrap();
        let rows = if date == week.start_date {
            json!({
                "stable": [["other-model", ["1", "1", null, "0"]], [null, ["0.5", "1", "0", "0"]]],
                "other-key": [["other-model", ["100", "10", "20", "0"]]]
            })
        } else {
            json!({})
        };
        snapshot(&mut host, "model-ranking", &day, Some(rows)).await;
    }

    let (cb, _, payload) = callback(&mut host, usage::STATISTICS_QUERY).await;
    let query: usage::QueryUsageStatistics = serde_json::from_slice(&payload).unwrap();
    assert_eq!(query.group_by, usage::UsageGrouping::KeyModel);
    assert_eq!(
        query.filter.records.client_key_ref.as_deref(),
        Some("stable")
    );
    assert_eq!(
        query.filter.records.start_ms,
        Window::day(first_live, timezone).unwrap().start_ms
    );
    assert_eq!(query.filter.records.end_ms, week.end_ms);

    let opened = usage::UsageReadOpened {
        reader: Some("models".into()),
        batch: batch(
            vec![
                statistics_row(None, &[], None),
                model_statistics_row(None, false, Some("0.5"), None),
                model_statistics_row(Some("other-model"), false, Some("0.0000000001"), None),
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
    let (cb, _, _) = callback(&mut host, usage::READ_NEXT).await;
    let last = batch(
        vec![
            model_statistics_row(None, true, None, None),
            model_statistics_row(Some("zero"), false, Some("0"), None),
            model_statistics_row(Some("unknown"), false, None, None),
        ],
        true,
    );
    host_reply(&mut host, cb, json!({}), serde_json::to_vec(&last).unwrap()).await;

    let (meta, body) = result(&mut host, 1).await;
    assert_eq!(meta["status"], 200);
    assert_eq!(
        body,
        json!({
            "period": "week", "startDate": week.start_date.to_string(),
            "endDate": week.end_date.to_string(), "timezone": "UTC",
            "models": [
                {"model":"other-model","usd":"1.0000000001","requests":"9007199254740994","tokens":null,"unknownCosts":"2"},
                {"model":null,"usd":"1","requests":"18014398509481987","tokens":"0","unknownCosts":"4"},
                {"model":"zero","usd":"0","requests":"9007199254740993","tokens":null,"unknownCosts":"2"},
                {"model":"unknown","usd":null,"requests":"9007199254740993","tokens":null,"unknownCosts":"2"}
            ]
        })
    );
    stop(host, server).await;
}
