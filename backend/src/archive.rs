//! 按自然日独立保存 Key 总量和模型明细，在宿主清理后继续排行

use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    time::{Duration, Instant},
};

use chrono::{DateTime, Days, NaiveDate, TimeDelta, Utc};
use chrono_tz::Tz;
use gateway_plugin_sdk::{
    ErrorCode, PluginFault,
    call::{
        host::{
            STATE_GET, STATE_PUT, StateGetRequest, StateGetResult, StatePutRequest, StatePutResult,
        },
        services::{settings, usage::UsageReadObservation},
    },
    client::{Empty, HostClient, SessionError, TypedCall, TypedReply},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::{
    calendar::Window,
    ranking::{self, Metrics, ModelRow, ModelTotals, RankingError, Row, add},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Archive {
    Keys,
    Models,
}

impl Archive {
    fn namespace(self) -> &'static str {
        match self {
            Self::Keys => "monthly-ranking",
            Self::Models => "model-ranking",
        }
    }
}

type KeyRows = BTreeMap<String, Metrics>;
type ModelRows = BTreeMap<String, Vec<(Option<String>, Metrics)>>;
const BATCH_DAYS: usize = 3;
const SCAN_INTERVAL: Duration = Duration::from_secs(2 * 60 * 60);
const RETRY_INTERVAL: Duration = Duration::from_secs(15 * 60);
const SETTLE_MS: i64 = TimeDelta::hours(2).num_milliseconds();
const REFRESH_LEAD_MS: i64 = TimeDelta::days(2).num_milliseconds();
const MAX_LAG_SECONDS: u64 = 60;
const BACKFILL_DAYS: u64 = 62;

#[derive(Default)]
pub(crate) struct Maintenance {
    schedule: Mutex<Schedule>,
}

struct Schedule {
    next_scan: Instant,
    pending: VecDeque<(Window, Phase, Archive)>,
    checked: BTreeMap<(NaiveDate, Archive), Phase>,
    retention_days: u32,
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            next_scan: Instant::now(),
            pending: VecDeque::new(),
            checked: BTreeMap::new(),
            retention_days: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Phase {
    Capture,
    Refresh,
}

impl Maintenance {
    pub(crate) async fn reconcile(
        &self,
        call: TypedCall<Empty>,
        timezone: Tz,
    ) -> Result<TypedReply<Empty>, PluginFault> {
        // 同一进程只推进一个扫描。跨进程写入由私有状态版本保护。
        let Ok(mut schedule) = self.schedule.try_lock() else {
            return Ok(TypedReply::new(Empty::default()));
        };
        schedule.run(&call.host, timezone).await?;
        Ok(TypedReply::new(Empty::default()))
    }
}

impl Schedule {
    async fn run(&mut self, host: &HostClient, timezone: Tz) -> Result<(), PluginFault> {
        if self.pending.is_empty() {
            if Instant::now() < self.next_scan {
                return Ok(());
            }
            self.next_scan = Instant::now() + RETRY_INTERVAL;
            let days = retention_days(host).await?;
            if self.retention_days != days {
                self.checked.clear();
                self.retention_days = days;
            }
            self.plan(timezone, Utc::now()).map_err(ranking_fault)?;
            self.next_scan = Instant::now() + SCAN_INTERVAL;
        }

        let mut fault = None;
        for _ in 0..BATCH_DAYS {
            let Some((window, phase, archive)) = self.pending.pop_front() else {
                break;
            };
            match capture(host, &window, timezone, self.retention_days, phase, archive).await {
                Ok(()) => {
                    self.checked.insert((window.start_date, archive), phase);
                }
                Err(error) => {
                    self.next_scan = self.next_scan.min(Instant::now() + RETRY_INTERVAL);
                    if archive == Archive::Keys
                        && matches!(error.code, ErrorCode::Upstream | ErrorCode::Cancelled)
                    {
                        // Key 统计回填会影响整轮，等到下一次扫描再读宿主
                        self.pending.clear();
                        return Err(error);
                    }
                    if error.code != ErrorCode::Conflict {
                        fault.get_or_insert(error);
                    }
                }
            }
        }
        fault.map_or(Ok(()), Err)
    }

    fn plan(&mut self, timezone: Tz, now: DateTime<Utc>) -> Result<(), RankingError> {
        let today = now.with_timezone(&timezone).date_naive();
        let retention = TimeDelta::days(i64::from(self.retention_days));
        let cutoff = now.checked_sub_signed(retention);
        let recent = today
            .checked_sub_days(Days::new(BACKFILL_DAYS))
            .unwrap_or(NaiveDate::MIN);
        let near_expiry = cutoff
            .map(|cutoff| cutoff.with_timezone(&timezone).date_naive())
            .unwrap_or(NaiveDate::MIN);

        // 回填最近历史。长保留期也在旧数据即将过期时补齐或刷新。
        self.checked.retain(|(date, _), _| *date >= near_expiry);
        let dates = near_expiry
            .iter_days()
            .take(4)
            .take_while(|date| *date < recent)
            .chain(
                near_expiry
                    .max(recent)
                    .iter_days()
                    .take_while(|date| *date < today),
            );
        for date in dates {
            let window = day_window(date, timezone)?;
            let Some(phase) = capture_phase(&window, now.timestamp_millis(), self.retention_days)
            else {
                continue;
            };
            for archive in [Archive::Keys, Archive::Models] {
                if self
                    .checked
                    .get(&(date, archive))
                    .is_none_or(|checked| *checked < phase)
                {
                    self.pending.push_back((window.clone(), phase, archive));
                }
            }
        }
        // 给即将过期的 Key 总量留出与独立归档前相同的处理顺序
        self.pending
            .make_contiguous()
            .sort_by_key(|(_, _, archive)| *archive);
        Ok(())
    }
}

fn capture_phase(window: &Window, now_ms: i64, days: u32) -> Option<Phase> {
    if window.start_ms == window.end_ms {
        return None;
    }
    let retention_ms = TimeDelta::days(i64::from(days)).num_milliseconds();
    let expiry_ms = window.start_ms.saturating_add(retention_ms);
    // 短保留期按可用时间缩短等待，且不保存任何可能已清理的自然日。
    let slack_ms = expiry_ms.saturating_sub(window.end_ms);
    if slack_ms <= 0
        || now_ms >= expiry_ms
        || now_ms < window.end_ms.saturating_add(SETTLE_MS.min(slack_ms / 4))
    {
        return None;
    }
    let refresh_ms = expiry_ms - REFRESH_LEAD_MS.min(slack_ms / 2);
    Some(if now_ms >= refresh_ms {
        Phase::Refresh
    } else {
        Phase::Capture
    })
}

async fn capture(
    host: &HostClient,
    window: &Window,
    timezone: Tz,
    days: u32,
    phase: Phase,
    archive: Archive,
) -> Result<(), PluginFault> {
    if !window.is_retained_at(Utc::now().timestamp_millis(), days) {
        return Err(ranking_fault(RankingError::HistoryUnavailable));
    }
    let prior = read_snapshot::<Value>(host, window, timezone, archive).await?;
    if phase == Phase::Capture && prior.is_some() {
        return Ok(());
    }

    let rows = match archive {
        Archive::Keys => {
            let rows = ranking::read_rows(host, window, days, archive_ready)
                .await
                .map_err(ranking_fault)?;
            let rows: KeyRows = rows
                .into_values()
                .map(|row| {
                    (
                        row.key_id,
                        Metrics(row.usd, row.requests, row.tokens, row.unknown_costs),
                    )
                })
                .collect();
            serde_json::to_value(rows)
        }
        Archive::Models => {
            let rows = ranking::read_models(host, window, days, None, archive_ready)
                .await
                .map_err(ranking_fault)?;
            let rows: ModelRows = rows
                .into_iter()
                .map(|(id, models)| {
                    let mut models: Vec<_> = models
                        .into_iter()
                        .map(|(model, total)| (model, Metrics::from(total)))
                        .collect();
                    models.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));
                    (id, models)
                })
                .collect();
            serde_json::to_value(rows)
        }
    }
    .map_err(invalid_state)?;
    let snapshot = Snapshot {
        start_ms: window.start_ms,
        end_ms: window.end_ms,
        rows,
    };
    if prior.as_ref().is_some_and(|(old, _)| *old == snapshot) {
        return Ok(());
    }
    let value = serde_json::to_value(snapshot).map_err(invalid_state)?;
    let _: StatePutResult = state_call(
        host,
        STATE_PUT,
        &StatePutRequest {
            namespace: archive.namespace().to_owned(),
            key: state_key(window.start_date, timezone),
            value,
            expected_version: prior.map(|(_, version)| version),
        },
    )
    .await?;
    Ok(())
}

fn archive_ready(observation: &UsageReadObservation, window: &Window) -> bool {
    observation
        .analytics
        .as_ref()
        .is_some_and(|analytics| match analytics.state.as_str() {
            // 空闲宿主的 coverage_end 可以早于自然日结束。ready 表示没有投影积压。
            "ready" => true,
            // 忙碌宿主允许近期队列活动，但历史时段应已过投影水位。
            "lagging" => {
                analytics.lag_seconds <= MAX_LAG_SECONDS
                    && analytics
                        .coverage_end_ms
                        .is_some_and(|end| end >= window.end_ms)
                    && observation
                        .generated_at_ms
                        .saturating_sub((analytics.lag_seconds as i64 + 1) * 1000)
                        >= window.end_ms
            }
            _ => false,
        })
}

pub(crate) async fn load_month(
    host: &HostClient,
    window: &Window,
    timezone: Tz,
    now: DateTime<Utc>,
) -> Result<Vec<Row>, RankingError> {
    let days = retention_days(host).await.map_err(state_error)?;
    let (archived, live) = split_retention(window, timezone, now, days)?;
    if archived.is_empty()
        && let Some(live) = &live
    {
        let rows = ranking::read_rows(host, live, days, |_, _| true).await?;
        return ranking::finish(host, rows).await;
    }

    let mut totals = HashMap::new();
    for day in archived {
        let (snapshot, _) = read_snapshot::<KeyRows>(host, &day, timezone, Archive::Keys)
            .await
            .map_err(state_error)?
            .ok_or(RankingError::HistoryUnavailable)?;
        for (id, metrics) in snapshot.rows {
            add(&mut totals, id, metrics.try_into()?)?;
        }
    }
    if let Some(live) = live {
        let rows = ranking::read_rows(host, &live, days, |_, _| true).await?;
        for (id, row) in rows {
            add(&mut totals, id, row.into())?;
        }
    }
    let rows = totals
        .into_iter()
        .map(|(key_id, total)| (key_id.clone(), Row::from_total(key_id, total)))
        .collect();
    ranking::finish(host, rows).await
}

pub(crate) async fn load_models(
    host: &HostClient,
    window: &Window,
    timezone: Tz,
    now: DateTime<Utc>,
    key_id: &str,
) -> Result<Vec<ModelRow>, RankingError> {
    if window.start_ms == window.end_ms {
        return Ok(Vec::new());
    }
    let days = retention_days(host).await.map_err(state_error)?;
    let (archived, live) = split_retention(window, timezone, now, days)?;
    let mut models = ModelTotals::new();
    for day in archived {
        if let Some((mut snapshot, _)) =
            read_snapshot::<ModelRows>(host, &day, timezone, Archive::Models)
                .await
                .map_err(state_error)?
        {
            for (model, metrics) in snapshot.rows.remove(key_id).into_iter().flatten() {
                add(&mut models, model, metrics.try_into()?)?;
            }
        } else {
            // 旧归档只能证明某个 Key 没有用量，不能把其总量归入未知模型
            let (snapshot, _) = read_snapshot::<KeyRows>(host, &day, timezone, Archive::Keys)
                .await
                .map_err(state_error)?
                .ok_or(RankingError::HistoryUnavailable)?;
            if snapshot.rows.contains_key(key_id) {
                return Err(RankingError::HistoryUnavailable);
            }
        }
    }
    if let Some(live) = live {
        let mut keys = ranking::read_models(host, &live, days, Some(key_id), |_, _| true).await?;
        for (model, total) in keys.remove(key_id).into_iter().flatten() {
            add(&mut models, model, total)?;
        }
    }
    Ok(ranking::finish_models(models))
}

fn split_retention(
    window: &Window,
    timezone: Tz,
    now: DateTime<Utc>,
    days: u32,
) -> Result<(Vec<Window>, Option<Window>), RankingError> {
    let now_ms = now.timestamp_millis();
    if window.is_retained_at(now_ms, days) {
        return Ok((Vec::new(), Some(window.clone())));
    }
    let mut archived = Vec::new();
    for date in window
        .start_date
        .iter_days()
        .take_while(|date| *date <= window.end_date)
    {
        let day = day_window(date, timezone)?;
        if day.start_ms == day.end_ms {
            continue;
        }
        if day.is_retained_at(now_ms, days) {
            return Ok((
                archived,
                Some(Window {
                    start_date: date,
                    start_ms: day.start_ms,
                    ..window.clone()
                }),
            ));
        }
        archived.push(day);
    }
    Ok((archived, None))
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot<R> {
    start_ms: i64,
    end_ms: i64,
    rows: R,
}

fn day_window(date: NaiveDate, timezone: Tz) -> Result<Window, RankingError> {
    Window::day(date, timezone).ok_or(RankingError::InvalidStatistics)
}

fn state_key(date: NaiveDate, timezone: Tz) -> String {
    format!("{}/{date}", timezone.name())
}

async fn read_snapshot<R: DeserializeOwned>(
    host: &HostClient,
    window: &Window,
    timezone: Tz,
    archive: Archive,
) -> Result<Option<(Snapshot<R>, u64)>, PluginFault> {
    let result: StateGetResult = state_call(
        host,
        STATE_GET,
        &StateGetRequest {
            namespace: archive.namespace().to_owned(),
            key: state_key(window.start_date, timezone),
        },
    )
    .await?;
    result
        .record
        .map(|record| {
            let snapshot: Snapshot<R> =
                serde_json::from_value(record.value).map_err(invalid_state)?;
            if record.schema_version != 1
                || snapshot.start_ms != window.start_ms
                || snapshot.end_ms != window.end_ms
            {
                return Err(PluginFault::new(
                    ErrorCode::InvalidInput,
                    "归档自然日边界或版本不匹配",
                ));
            }
            Ok((snapshot, record.version))
        })
        .transpose()
}

async fn retention_days(host: &HostClient) -> Result<u32, PluginFault> {
    host.service::<settings::Load>(())
        .await
        .map(|settings| settings.usage_retention_days)
        .map_err(|_| ranking_fault(RankingError::ProjectionUnavailable))
}

async fn state_call<I: Serialize, O: DeserializeOwned>(
    host: &HostClient,
    method: &str,
    request: &I,
) -> Result<O, PluginFault> {
    let reply = host
        .call(
            method,
            serde_json::to_value(request).map_err(invalid_state)?,
            Vec::new(),
        )
        .await
        .map_err(SessionError::into_plugin_fault)?;
    serde_json::from_value(reply.result).map_err(invalid_state)
}

fn invalid_state(_: serde_json::Error) -> PluginFault {
    PluginFault::new(ErrorCode::InvalidInput, "用量归档状态无效")
}

fn state_error(fault: PluginFault) -> RankingError {
    match fault.code {
        ErrorCode::Timeout => RankingError::Timeout,
        ErrorCode::InvalidInput => RankingError::InvalidStatistics,
        ErrorCode::Capacity => RankingError::TooLarge,
        _ => RankingError::ProjectionUnavailable,
    }
}

fn ranking_fault(error: RankingError) -> PluginFault {
    let code = match error {
        RankingError::HistoryUnavailable => ErrorCode::Rejected,
        RankingError::ProjectionUnavailable | RankingError::KeyNamesUnavailable => {
            ErrorCode::Upstream
        }
        RankingError::InvalidStatistics => ErrorCode::InvalidInput,
        RankingError::TooLarge => ErrorCode::Capacity,
        RankingError::Timeout => ErrorCode::Timeout,
    };
    PluginFault::new(code, "无法保存自然日用量归档")
}

#[cfg(test)]
mod tests {
    use super::*;
    use gateway_plugin_sdk::call::services::usage::UsageReadAnalytics;
    use ranking::{Totals, format_usd};

    #[test]
    fn catches_up_before_retention_and_refreshes_through_dst() {
        let zone = chrono_tz::America::New_York;
        let now: DateTime<Utc> = "2024-04-06T09:00:00Z".parse().unwrap();
        let mut schedule = Schedule {
            retention_days: 31,
            ..Schedule::default()
        };
        schedule.plan(zone, now).unwrap();
        let (first, phase, archive) = schedule.pending.front().unwrap();
        assert_eq!(first.start_date.to_string(), "2024-03-07");
        assert_eq!(*phase, Phase::Refresh);
        assert_eq!(*archive, Archive::Keys);
        let (last, phase, _) = schedule.pending.back().unwrap();
        assert_eq!(last.start_date.to_string(), "2024-04-05");
        assert_eq!(*phase, Phase::Capture);
        for (window, phase, archive) in schedule.pending.drain(..) {
            schedule.checked.insert((window.start_date, archive), phase);
        }
        schedule.plan(zone, now).unwrap();
        assert!(schedule.pending.is_empty());

        let spring = day_window("2024-03-10".parse().unwrap(), zone).unwrap();
        assert_eq!(
            spring.end_ms - spring.start_ms,
            TimeDelta::hours(23).num_milliseconds()
        );
        let mut later = now + TimeDelta::days(2);
        assert_eq!(
            capture_phase(&spring, later.timestamp_millis(), 31),
            Some(Phase::Refresh)
        );
        later += TimeDelta::days(2);
        assert_eq!(capture_phase(&spring, later.timestamp_millis(), 31), None);
    }

    #[test]
    fn idle_ready_and_recent_lag_can_archive_but_backfill_and_old_lag_cannot() {
        let now: DateTime<Utc> = "2024-04-01T03:00:00Z".parse().unwrap();
        let day = day_window("2024-03-31".parse().unwrap(), chrono_tz::UTC).unwrap();
        let observation = |state: &str, lag_seconds, coverage_end_ms| UsageReadObservation {
            generated_at_ms: now.timestamp_millis(),
            analytics: Some(UsageReadAnalytics {
                coverage_start_ms: None,
                coverage_end_ms,
                lag_seconds,
                state: state.into(),
                generation: 1,
            }),
            total_groups: None,
        };
        assert!(archive_ready(&observation("ready", 0, None), &day));
        assert!(archive_ready(
            &observation("lagging", 1, Some(day.end_ms)),
            &day
        ));
        assert!(!archive_ready(
            &observation("lagging", 61, Some(day.end_ms)),
            &day
        ));
        assert!(!archive_ready(
            &observation("lagging", 1, Some(day.end_ms - 1)),
            &day
        ));
        assert!(!archive_ready(
            &observation("building", 0, Some(day.end_ms)),
            &day
        ));
    }

    #[test]
    fn merge_keeps_exact_costs_large_counts_and_known_zero() {
        let mut totals = HashMap::<String, Totals>::new();
        let mut merge = |id: &str, json: &str| {
            let metrics: Metrics = serde_json::from_str(json).unwrap();
            add(&mut totals, id.into(), metrics.try_into().unwrap()).unwrap();
        };
        merge(
            "a",
            r#"["9999999999.9999999999","9007199254740993",null,"0"]"#,
        );
        merge("a", r#"["0.0000000002","2","0","2"]"#);
        merge("a", r#"[null,"1",null,"1"]"#);
        merge("b", r#"[null,"1",null,"1"]"#);
        merge("c", r#"["0","1","0","0"]"#);
        assert_eq!(
            totals["a"].usd.map(format_usd).as_deref(),
            Some("10000000000.0000000001")
        );
        assert_eq!(totals["a"].requests, 9_007_199_254_740_996);
        assert_eq!(totals["a"].tokens, Some(0));
        assert_eq!(totals["a"].unknown_costs, 3);
        assert_eq!((totals["b"].usd, totals["b"].tokens), (None, None));
        assert_eq!((totals["c"].usd, totals["c"].tokens), (Some(0), Some(0)));
    }
}
