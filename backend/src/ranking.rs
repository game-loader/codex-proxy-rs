//! 从宿主已发布的 Key 与模型统计生成排行

use std::collections::{HashMap, HashSet, hash_map::Entry};

use gateway_plugin_sdk::{
    ErrorCode, PluginFault,
    call::{
        host::KeyListRequest,
        services::{
            settings,
            usage::{
                QueryUsageStatistics, UsageCount, UsageGrouping, UsageMetric, UsageReadBatch,
                UsageReadFilter, UsageReadItem, UsageReadObservation, UsageRecordFilter,
                UsageStatisticsGroup, UsageStatisticsMetrics,
            },
        },
    },
    client::HostClient,
};
use serde::{Deserialize, Serialize};

use crate::calendar::Window;

pub(crate) const MAX_ROWS: usize = 8_192;
const KEY_PAGE_SIZE: u16 = 200;
const USD_SCALE: u128 = 10_000_000_000;

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
    let mut rows = HashMap::new();
    read_groups(
        host,
        window,
        days,
        UsageGrouping::Key,
        None,
        eligible,
        |group, metrics| {
            if !group.unassigned_contribution
                && let Some(id) = group.client_key_ref
            {
                let row = Row::new(id.clone(), metrics)?;
                insert_row(&mut rows, id, row)?;
            }
            Ok(())
        },
    )
    .await?;
    Ok(rows)
}

pub(crate) type ModelTotals = HashMap<Option<String>, Totals>;

pub(crate) async fn read_models(
    host: &HostClient,
    window: &Window,
    days: u32,
    key_id: Option<&str>,
    eligible: impl Fn(&UsageReadObservation, &Window) -> bool,
) -> Result<HashMap<String, ModelTotals>, RankingError> {
    let mut keys = HashMap::<String, ModelTotals>::new();
    let mut groups = 0;
    read_groups(
        host,
        window,
        days,
        UsageGrouping::KeyModel,
        key_id,
        eligible,
        |group, metrics| {
            let Some(id) = group.client_key_ref else {
                // 没有 Key 身份的贡献无法归入任何 Key
                return if key_id.is_some() {
                    Err(RankingError::InvalidStatistics)
                } else {
                    Ok(())
                };
            };
            if key_id.is_some_and(|key| key != id) {
                return Err(RankingError::InvalidStatistics);
            }
            if groups == MAX_ROWS {
                return Err(RankingError::TooLarge);
            }
            groups += 1;
            let total = Totals::try_from(Metrics::from_statistics(metrics)?)?;
            let models = keys.entry(id).or_default();
            if group.model.is_none() {
                // 原生记录和外部贡献可以分别产生缺少模型的分组
                add(models, None, total)?;
            } else {
                insert_row(models, group.model, total)?;
            }
            Ok(())
        },
    )
    .await?;
    Ok(keys)
}

async fn read_groups(
    host: &HostClient,
    window: &Window,
    days: u32,
    group_by: UsageGrouping,
    key_id: Option<&str>,
    eligible: impl Fn(&UsageReadObservation, &Window) -> bool,
    push: impl FnMut(UsageStatisticsGroup, UsageStatisticsMetrics) -> Result<(), RankingError>,
) -> Result<(), RankingError> {
    if window.start_ms == window.end_ms {
        return Ok(());
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
                    client_key_ref: key_id.map(str::to_owned),
                },
                routing_group_ref: None,
            },
            group_by,
            batch_size: Some(256),
        })
        .await
        .map_err(statistics_fault)?;

    let mut state = BatchState::default();
    let mut push = push;
    while let Some(batch) = reader.next_batch().await.map_err(statistics_fault)? {
        let result = if eligible(&batch.observation, window) {
            state.push(batch, window, days, &mut push)
        } else {
            Err(RankingError::ProjectionUnavailable)
        };
        if let Err(error) = result {
            let _ = reader.close().await;
            return Err(error);
        }
    }
    if state.complete {
        Ok(())
    } else {
        Err(RankingError::InvalidStatistics)
    }
}

fn insert_row<K: Eq + std::hash::Hash, V>(
    rows: &mut HashMap<K, V>,
    key: K,
    row: V,
) -> Result<(), RankingError> {
    if rows.len() == MAX_ROWS {
        return Err(RankingError::TooLarge);
    }
    if rows.insert(key, row).is_some() {
        return Err(RankingError::InvalidStatistics);
    }
    Ok(())
}

pub(crate) async fn finish(
    host: &HostClient,
    mut rows: HashMap<String, Row>,
) -> Result<Vec<Row>, RankingError> {
    if !rows.is_empty() {
        load_names(host, &mut rows).await?;
    }
    let mut rows: Vec<_> = rows.into_values().collect();
    rows.sort_unstable_by(|a, b| {
        b.scaled_usd
            .cmp(&a.scaled_usd)
            .then_with(|| a.key_id.cmp(&b.key_id))
    });
    Ok(rows)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelRow {
    model: Option<String>,
    usd: Option<String>,
    requests: UsageCount,
    tokens: Option<UsageCount>,
    unknown_costs: UsageCount,
}

pub(crate) fn finish_models(models: ModelTotals) -> Vec<ModelRow> {
    let mut models: Vec<_> = models.into_iter().collect();
    models.sort_unstable_by(|(a_model, a), (b_model, b)| {
        b.usd.cmp(&a.usd).then_with(|| a_model.cmp(b_model))
    });
    models
        .into_iter()
        .map(|(model, totals)| {
            let Metrics(usd, requests, tokens, unknown_costs) = totals.into();
            ModelRow {
                model,
                usd,
                requests,
                tokens,
                unknown_costs,
            }
        })
        .collect()
}

fn check_retention(window: &Window, now_ms: i64, days: u32) -> Result<(), RankingError> {
    if window.is_retained_at(now_ms, days) {
        Ok(())
    } else {
        Err(RankingError::HistoryUnavailable)
    }
}

#[derive(Default)]
struct BatchState {
    groups_read: u64,
    saw_total: bool,
    complete: bool,
}

impl BatchState {
    fn push(
        &mut self,
        batch: UsageReadBatch,
        window: &Window,
        days: u32,
        mut push: impl FnMut(UsageStatisticsGroup, UsageStatisticsMetrics) -> Result<(), RankingError>,
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
            push(group, row.metrics)?;
        }
        if batch.complete {
            if !self.saw_total || observation.total_groups != Some(UsageCount(self.groups_read)) {
                return Err(RankingError::InvalidStatistics);
            }
            self.complete = true;
        }
        Ok(())
    }
}

impl Row {
    pub(crate) fn from_total(key_id: String, total: Totals) -> Self {
        let scaled_usd = total.usd;
        let Metrics(usd, requests, tokens, unknown_costs) = total.into();
        Self {
            key_name: key_id.clone(),
            key_id,
            usd,
            requests,
            tokens,
            unknown_costs,
            scaled_usd,
        }
    }

    fn new(key_id: String, metrics: UsageStatisticsMetrics) -> Result<Self, RankingError> {
        let Metrics(usd, requests, tokens, unknown_costs) = Metrics::from_statistics(metrics)?;
        let scaled_usd = usd
            .as_deref()
            .map(|amount| scaled_amount(amount).ok_or(RankingError::InvalidStatistics))
            .transpose()?;
        Ok(Self {
            key_name: key_id.clone(),
            key_id,
            usd,
            requests,
            tokens,
            unknown_costs,
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

fn scaled_amount(amount: &str) -> Option<u128> {
    // 保留宿主十位小数精度，合并后的整数位可增长至 u128 上限
    let (whole, fraction) = amount.split_once('.').unwrap_or((amount, ""));
    let digits = |value: &str| value.bytes().all(|b| b.is_ascii_digit());
    if !digits(whole) || fraction.len() > 10 || !digits(fraction) {
        return None;
    }
    let whole: u128 = whole.parse().ok()?;
    let fractional = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u128>().ok()? * 10_u128.pow(10 - fraction.len() as u32)
    };
    whole.checked_mul(USD_SCALE)?.checked_add(fractional)
}

// 费用为原始十进制字符串，计数复用 SDK 的无损字符串编码。
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Metrics(
    pub(crate) Option<String>,
    pub(crate) UsageCount,
    pub(crate) Option<UsageCount>,
    pub(crate) UsageCount,
);

impl Metrics {
    fn from_statistics(metrics: UsageStatisticsMetrics) -> Result<Self, RankingError> {
        let usd = metrics
            .costs
            .into_iter()
            .find(|cost| cost.currency == "USD")
            .map(|cost| cost.amount);
        let requests = *metrics
            .values
            .get(&UsageMetric::RequestCount)
            .ok_or(RankingError::InvalidStatistics)?;
        let tokens = metrics.values.get(&UsageMetric::TotalTokens).copied();
        Ok(Self(
            usd,
            requests,
            tokens,
            metrics.cost_coverage.unavailable,
        ))
    }
}

impl From<Totals> for Metrics {
    fn from(total: Totals) -> Self {
        Self(
            total.usd.map(format_usd),
            UsageCount(total.requests),
            total.tokens.map(UsageCount),
            UsageCount(total.unknown_costs),
        )
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Totals {
    pub(crate) usd: Option<u128>,
    pub(crate) requests: u64,
    pub(crate) tokens: Option<u64>,
    pub(crate) unknown_costs: u64,
}

impl TryFrom<Metrics> for Totals {
    type Error = RankingError;

    fn try_from(metrics: Metrics) -> Result<Self, RankingError> {
        let Metrics(usd, requests, tokens, unknown_costs) = metrics;
        Ok(Self {
            usd: usd
                .as_deref()
                .map(|amount| scaled_amount(amount).ok_or(RankingError::InvalidStatistics))
                .transpose()?,
            requests: requests.get(),
            tokens: tokens.map(UsageCount::get),
            unknown_costs: unknown_costs.get(),
        })
    }
}

impl From<Row> for Totals {
    fn from(row: Row) -> Self {
        Self {
            usd: row.scaled_usd,
            requests: row.requests.get(),
            tokens: row.tokens.map(UsageCount::get),
            unknown_costs: row.unknown_costs.get(),
        }
    }
}

pub(crate) fn add<K: Eq + std::hash::Hash>(
    totals: &mut HashMap<K, Totals>,
    id: K,
    next: Totals,
) -> Result<(), RankingError> {
    let full = totals.len() >= MAX_ROWS;
    match totals.entry(id) {
        Entry::Vacant(entry) if !full => {
            entry.insert(next);
        }
        Entry::Vacant(_) => return Err(RankingError::TooLarge),
        Entry::Occupied(mut entry) => {
            let total = entry.get_mut();
            let overflow = RankingError::InvalidStatistics;
            *total = Totals {
                usd: match (total.usd, next.usd) {
                    (Some(a), Some(b)) => Some(a.checked_add(b).ok_or(overflow)?),
                    (a, b) => a.or(b),
                },
                requests: total.requests.checked_add(next.requests).ok_or(overflow)?,
                tokens: match (total.tokens, next.tokens) {
                    (Some(a), Some(b)) => Some(a.checked_add(b).ok_or(overflow)?),
                    (a, b) => a.or(b),
                },
                unknown_costs: total
                    .unknown_costs
                    .checked_add(next.unknown_costs)
                    .ok_or(overflow)?,
            };
        }
    }
    Ok(())
}

pub(crate) fn format_usd(amount: u128) -> String {
    let whole = amount / USD_SCALE;
    let fraction = amount % USD_SCALE;
    if fraction == 0 {
        return whole.to_string();
    }
    let fraction = format!("{fraction:010}");
    format!("{whole}.{}", fraction.trim_end_matches('0'))
}
