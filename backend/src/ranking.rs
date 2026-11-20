//! 从宿主已发布的 Key 分组统计生成排行

use std::collections::{HashMap, HashSet};

use gateway_plugin_sdk::{
    ErrorCode, PluginFault,
    call::{
        host::KeyListRequest,
        services::{
            settings,
            usage::{
                QueryUsageStatistics, UsageCount, UsageGrouping, UsageMetric, UsageReadBatch,
                UsageReadFilter, UsageReadItem, UsageReadObservation, UsageRecordFilter,
                UsageStatisticsMetrics,
            },
        },
    },
    client::HostClient,
};
use serde::Serialize;

use crate::calendar::Window;

pub(crate) const MAX_ROWS: usize = 8_192;
const KEY_PAGE_SIZE: u16 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RankingError {
    HistoryUnavailable,
    ProjectionUnavailable,
    KeyNamesUnavailable,
    InvalidStatistics,
    TooLarge,
    Timeout,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Row {
    pub(crate) key_id: String,
    pub(crate) key_name: String,
    pub(crate) usd: Option<String>,
    pub(crate) requests: UsageCount,
    pub(crate) tokens: Option<UsageCount>,
    pub(crate) unknown_costs: UsageCount,
    #[serde(skip)]
    pub(crate) scaled_usd: Option<u128>,
}

pub(crate) async fn load(
    host: &HostClient,
    window: &Window,
    now_ms: i64,
) -> Result<Vec<Row>, RankingError> {
    if window.start_ms == window.end_ms {
        return Ok(Vec::new());
    }
    let days = host
        .service::<settings::Load>(())
        .await
        .map_err(|_| RankingError::ProjectionUnavailable)?
        .usage_retention_days;
    check_retention(window, now_ms, days)?;

    let rows = read_rows(host, window, days, |_, _| true).await?;
    finish(host, rows).await
}

pub(crate) async fn read_rows(
    host: &HostClient,
    window: &Window,
    days: u32,
    eligible: impl Fn(&UsageReadObservation, &Window) -> bool,
) -> Result<HashMap<String, Row>, RankingError> {
    if window.start_ms == window.end_ms {
        return Ok(HashMap::new());
    }
    let mut reader = host
        .query_usage_statistics(QueryUsageStatistics {
            filter: UsageReadFilter {
                records: UsageRecordFilter {
                    start_ms: window.start_ms,
                    end_ms: window.end_ms,
                    provider: None,
                    model: None,
                    account_ref: None,
                    client_key_ref: None,
                },
                routing_group_ref: None,
            },
            group_by: UsageGrouping::Key,
            batch_size: Some(256),
        })
        .await
        .map_err(statistics_fault)?;

    let mut ranking = Ranking::default();
    while let Some(batch) = reader.next_batch().await.map_err(statistics_fault)? {
        if !eligible(&batch.observation, window) {
            let _ = reader.close().await;
            return Err(RankingError::ProjectionUnavailable);
        }
        if let Err(error) = ranking.push(batch, window, days) {
            let _ = reader.close().await;
            return Err(error);
        }
    }
    if !ranking.complete {
        return Err(RankingError::InvalidStatistics);
    }
    Ok(ranking.rows)
}

pub(crate) async fn finish(
    host: &HostClient,
    mut rows: HashMap<String, Row>,
) -> Result<Vec<Row>, RankingError> {
    if !rows.is_empty() {
        load_names(host, &mut rows).await?;
    }
    Ok(Ranking {
        rows,
        ..Ranking::default()
    }
    .sorted_rows())
}

fn check_retention(window: &Window, now_ms: i64, days: u32) -> Result<(), RankingError> {
    if window.is_retained_at(now_ms, days) {
        Ok(())
    } else {
        Err(RankingError::HistoryUnavailable)
    }
}

#[derive(Default)]
struct Ranking {
    rows: HashMap<String, Row>,
    groups_read: u64,
    saw_total: bool,
    complete: bool,
}

impl Ranking {
    fn push(
        &mut self,
        batch: UsageReadBatch,
        window: &Window,
        days: u32,
    ) -> Result<(), RankingError> {
        let observation = batch.observation;
        check_retention(window, observation.generated_at_ms, days)?;
        let analytics = observation
            .analytics
            .ok_or(RankingError::InvalidStatistics)?;
        // 完成回填后的 coverage_start 是最早现存记录，并非无流量时段的起点
        if !matches!(analytics.state.as_str(), "ready" | "lagging") {
            return Err(RankingError::ProjectionUnavailable);
        }
        for item in batch.items {
            let UsageReadItem::Statistics(row) = item else {
                return Err(RankingError::InvalidStatistics);
            };
            if !self.saw_total {
                if row.group.is_some() {
                    return Err(RankingError::InvalidStatistics);
                }
                self.saw_total = true;
                continue;
            }
            let group = row.group.ok_or(RankingError::InvalidStatistics)?;
            self.groups_read += 1;
            if group.unassigned_contribution {
                continue;
            }
            if let Some(id) = group.client_key_ref {
                if self.rows.len() == MAX_ROWS {
                    return Err(RankingError::TooLarge);
                }
                let row = Row::new(id.clone(), row.metrics)?;
                if self.rows.insert(id, row).is_some() {
                    return Err(RankingError::InvalidStatistics);
                }
            }
        }
        if batch.complete {
            if !self.saw_total || observation.total_groups != Some(UsageCount(self.groups_read)) {
                return Err(RankingError::InvalidStatistics);
            }
            self.complete = true;
        }
        Ok(())
    }

    fn sorted_rows(self) -> Vec<Row> {
        let mut rows: Vec<_> = self.rows.into_values().collect();
        rows.sort_unstable_by(|a, b| {
            b.scaled_usd
                .cmp(&a.scaled_usd)
                .then_with(|| a.key_id.cmp(&b.key_id))
        });
        rows
    }
}

impl Row {
    fn new(key_id: String, metrics: UsageStatisticsMetrics) -> Result<Self, RankingError> {
        let usd = metrics
            .costs
            .into_iter()
            .find(|cost| cost.currency == "USD")
            .map(|cost| cost.amount);
        let scaled_usd = usd
            .as_deref()
            .map(|amount| scaled_amount(amount).ok_or(RankingError::InvalidStatistics))
            .transpose()?;
        Ok(Self {
            key_name: key_id.clone(),
            key_id,
            usd,
            requests: *metrics
                .values
                .get(&UsageMetric::RequestCount)
                .ok_or(RankingError::InvalidStatistics)?,
            tokens: metrics.values.get(&UsageMetric::TotalTokens).copied(),
            unknown_costs: metrics.cost_coverage.unavailable,
            scaled_usd,
        })
    }
}

async fn load_names(
    host: &HostClient,
    rows: &mut HashMap<String, Row>,
) -> Result<(), RankingError> {
    let mut cursor = None;
    let mut seen_cursors = HashSet::new();
    loop {
        let page = host
            .list_keys(KeyListRequest {
                cursor,
                limit: KEY_PAGE_SIZE,
            })
            .await
            .map_err(|fault| match fault.code {
                ErrorCode::Timeout => RankingError::Timeout,
                _ => RankingError::KeyNamesUnavailable,
            })?;
        for key in page.keys {
            if let Some(row) = rows.get_mut(&key.id) {
                row.key_name = key.name;
            }
        }
        let Some(next) = page.next_cursor else {
            return Ok(());
        };
        if !seen_cursors.insert(next.clone()) {
            return Err(RankingError::KeyNamesUnavailable);
        }
        cursor = Some(next);
    }
}

fn statistics_fault(fault: PluginFault) -> RankingError {
    match fault.code {
        ErrorCode::Capacity => RankingError::TooLarge,
        ErrorCode::Timeout => RankingError::Timeout,
        _ => RankingError::ProjectionUnavailable,
    }
}

pub(crate) fn scaled_amount(amount: &str) -> Option<u128> {
    // 宿主 Decimal 为非负十位整数与十位小数，排行比较不经过浮点
    let (whole, fraction) = amount.split_once('.').unwrap_or((amount, ""));
    let digits = |value: &str| value.len() <= 10 && value.bytes().all(|b| b.is_ascii_digit());
    if !digits(whole) || !digits(fraction) {
        return None;
    }
    let whole: u128 = whole.parse().ok()?;
    let fractional = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u128>().ok()? * 10_u128.pow(10 - fraction.len() as u32)
    };
    Some(whole * 10_000_000_000 + fractional)
}
