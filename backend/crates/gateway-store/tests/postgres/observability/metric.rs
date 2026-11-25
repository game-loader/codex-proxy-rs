//! 单项洞察保留统计口径，未选择的分布和并发不进入结果

use super::*;
use admin_observability::{
    RequestHealth, TimeRange, UsageInsightMetric as Metric, UsageInsightsObservation,
};

#[tokio::test]
async fn global_health_reads_only_the_requested_window_across_keys() {
    let Some(database) = TestDatabase::create("global_request_health").await else {
        return;
    };
    let end = "2026-09-01T12:07:30Z"
        .parse::<chrono::DateTime<Utc>>()
        .unwrap();
    // 样本已有起点的成功、20 分钟前的成功和窗口内的服务失败
    seed_observability_facts(&database.pool, end).await.unwrap();
    sqlx::query(
        "insert into model_requests (
           id, client_api_key_ref, operation, client_transport, provider_kind,
           outcome, client_status_code, started_at, recovery_lease_expires_at, completed_at,
           request_observation_json
         )
         select id, 'other_key', 'responses', 'http_sse', 'openai', outcome, status,
           $1 + age, $1 + interval '1 hour',
           case when outcome <> 'running' then $1 + age + interval '1 second' end,
           '{\"request\":{\"configRevision\":1,\"protocol\":\"openai\",\"endpoint\":\"/v1/responses\",\"compact\":false},
              \"routing\":{\"scope\":\"all\",\"groupRefs\":[],\"groupNamesSnapshot\":[]}}'::jsonb
         from (values
           ('req_other_incomplete', 'incomplete', 200, interval '-2 minutes'),
           ('req_other_caller_error', 'failed', 429, interval '-3 minutes'),
           ('req_other_cancelled', 'cancelled', 499, interval '-4 minutes'),
           ('req_other_running', 'running', null, interval '-5 minutes'),
           ('req_other_recovered', 'failed', 502, interval '-12 minutes'),
           ('req_other_at_end', 'failed', 502, interval '0')
         ) as samples(id, outcome, status, age)",
    )
    .bind(end)
    .execute(&database.pool)
    .await
    .unwrap();
    sqlx::query(
        "update model_requests set continuation_requested = true,
           continuation_affinity_hash = repeat('a',64),
           recovery_request_id = 'req_observe_success', recovered_at = $1,
           recovery_attempt_count = 1,
           request_observation_json = request_observation_json ||
             '{\"recovery\":{\"retryDelayMs\":1000,\"totalLatencyMs\":2000}}'::jsonb
         where id = 'req_other_recovered'",
    )
    .bind(end)
    .execute(&database.pool)
    .await
    .unwrap();
    super::super::drain_analytics(&database.pool).await;

    let store = admin_observability_store(&database.pool);
    for (minutes, expected) in [
        (15, RequestHealth::from_counts(1, 2, 1, 1, 1)),
        (5, RequestHealth::from_counts(0, 1, 1, 1, 1)),
        (30, RequestHealth::from_counts(2, 2, 1, 1, 1)),
    ] {
        let range = TimeRange::new(end - TimeDelta::minutes(minutes), end).unwrap();
        let health = store.global_request_health(range).await.unwrap();
        assert_eq!(health, expected, "{minutes}-minute window");
    }
    database.close().await;
}

#[tokio::test]
async fn usage_metrics_preserve_selected_values_for_aggregate_and_exact_reads() {
    let Some(database) = TestDatabase::create("usage_metrics").await else {
        return;
    };
    let now = Utc::now();
    seed_observability_facts(&database.pool, now).await.unwrap();
    seed_calculated_billing_facts(&database.pool, now)
        .await
        .unwrap();
    super::super::drain_analytics(&database.pool).await;
    let store = admin_observability_store(&database.pool);
    let range =
        admin_observability::TimeRange::new(now - TimeDelta::hours(2), now + TimeDelta::hours(1))
            .unwrap();
    let granularity = admin_observability::Granularity::FifteenMinutes;
    for filter in [
        admin_observability::UsageFilter::default(),
        admin_observability::UsageFilter {
            model: Some("public-model".to_owned()),
            ..Default::default()
        },
    ] {
        let full = store
            .usage_insights_observation(range, filter.clone(), granularity, true)
            .await
            .unwrap();
        assert!(full.overview.requests.request_count > 0);
        for metric in [
            Metric::Health,
            Metric::Latency,
            Metric::FirstToken,
            Metric::Throughput,
            Metric::Load,
            Metric::Cost,
            Metric::Savings,
            Metric::Cache,
        ] {
            let selected = store
                .usage_metric_observation(range, filter.clone(), granularity, metric)
                .await
                .unwrap();
            assert_eq!(
                selected.overview.requests.request_count, full.overview.requests.request_count,
                "{metric:?}"
            );
            assert_eq!(selected.trend.len(), full.trend.len());
            assert_selected_metrics(&selected.overview.requests, &full.overview.requests, metric);
            for (actual, expected) in selected.trend.iter().zip(&full.trend) {
                assert_eq!(actual.bucket_start, expected.bucket_start);
                assert_eq!(actual.metrics.request_count, expected.metrics.request_count);
                assert_selected_metrics(&actual.metrics, &expected.metrics, metric);
            }
            assert_costs(&selected, &full, metric);
            if metric == Metric::Load {
                assert_eq!(selected.concurrency, full.concurrency);
            } else {
                assert!(selected.concurrency.is_empty(), "{metric:?}");
            }
        }
    }
    database.close().await;
}

fn assert_selected_metrics(
    actual: &admin_observability::RequestMetrics,
    expected: &admin_observability::RequestMetrics,
    metric: Metric,
) {
    match metric {
        Metric::Health => {
            assert_eq!(actual.success_count, expected.success_count);
            assert_eq!(actual.failure_count, expected.failure_count);
            assert_eq!(actual.cancelled_count, expected.cancelled_count);
            assert_eq!(actual.incomplete_count, expected.incomplete_count);
            assert_eq!(actual.caller_error_count, expected.caller_error_count);
        }
        Metric::Latency => assert_eq!(actual.latency_percentiles, expected.latency_percentiles),
        Metric::FirstToken => assert_eq!(
            actual.first_token_latency_percentiles,
            expected.first_token_latency_percentiles
        ),
        Metric::Throughput => assert_eq!(
            (
                actual.output_throughput_p10,
                actual.output_throughput_p50,
                actual.output_throughput_p90
            ),
            (
                expected.output_throughput_p10,
                expected.output_throughput_p50,
                expected.output_throughput_p90
            )
        ),
        Metric::Load => {
            assert_eq!(
                actual.admission_decision_percentiles,
                expected.admission_decision_percentiles
            );
            assert_eq!(
                actual.account_selection_wait_percentiles,
                expected.account_selection_wait_percentiles
            );
        }
        Metric::Cost | Metric::Savings => {
            assert_eq!(actual.success_count, expected.success_count);
        }
        Metric::Cache => {
            assert_eq!(actual.success_count, expected.success_count);
            assert_eq!(actual.input_tokens, expected.input_tokens);
            assert_eq!(actual.cached_input_tokens, expected.cached_input_tokens);
            assert_eq!(
                actual.cache_hit_request_count,
                expected.cache_hit_request_count
            );
            assert_eq!(
                actual.cache_eligible_request_count,
                expected.cache_eligible_request_count
            );
        }
    }
    if metric != Metric::Latency {
        assert_eq!(actual.latency_percentiles, Default::default());
    }
    if metric != Metric::FirstToken {
        assert_eq!(actual.first_token_latency_percentiles, Default::default());
    }
    if metric != Metric::Throughput {
        assert_eq!(actual.output_throughput_p50, None);
    }
    if metric != Metric::Load {
        assert_eq!(actual.admission_decision_percentiles, Default::default());
    }
}

fn assert_costs(
    actual: &UsageInsightsObservation,
    expected: &UsageInsightsObservation,
    metric: Metric,
) {
    if matches!(metric, Metric::Cost | Metric::Savings | Metric::Cache) {
        assert_eq!(
            actual.overview.attempts.costs,
            expected.overview.attempts.costs
        );
        assert_eq!(
            actual.overview.attempts.cost_coverage,
            expected.overview.attempts.cost_coverage
        );
        assert_eq!(actual.costs, expected.costs);
        for (actual, expected) in actual.trend.iter().zip(&expected.trend) {
            assert_eq!(actual.costs, expected.costs);
            assert_eq!(actual.cost_coverage, expected.cost_coverage);
        }
    } else {
        assert!(actual.overview.attempts.costs.is_empty());
        assert!(actual.costs.is_empty());
    }
}
