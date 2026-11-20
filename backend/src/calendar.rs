//! 按部署时区解析排行参数并计算自然日边界

use chrono::{DateTime, Datelike, Days, NaiveDate, TimeDelta, TimeZone, Utc};
use chrono_tz::{GapInfo, Tz};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Period {
    Day,
    Week,
    Month,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Query {
    period: Period,
    date: Option<String>,
}

#[derive(Debug)]
pub(crate) struct Window {
    pub period: Period,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub start_ms: i64,
    pub end_ms: i64,
}

impl Window {
    pub fn parse(query: &str, timezone: Tz, now: DateTime<Utc>) -> Result<Self, &'static str> {
        if query.len() > 128 {
            return Err("排行查询参数过长");
        }
        let query: Query =
            serde_urlencoded::from_str(query).map_err(|_| "请指定有效的 period，可选填 date")?;
        let date = match query.date {
            Some(date) => parse_date(&date).ok_or("date 必须是有效的 YYYY-MM-DD 日期")?,
            None => now.with_timezone(&timezone).date_naive(),
        };
        bounds(query.period, date, timezone).ok_or("排行日期超出可查询范围")
    }

    pub fn is_retained_at(&self, now_ms: i64, retention_days: u32) -> bool {
        // 宿主按实际经过的天数清理，查询仍以本地自然日作为边界
        let retained_ms = TimeDelta::days(i64::from(retention_days)).num_milliseconds();
        self.start_ms >= now_ms.saturating_sub(retained_ms)
    }
}

fn parse_date(value: &str) -> Option<NaiveDate> {
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()?;
    (value.len() == 10 && (1..=9999).contains(&date.year()) && date.to_string() == value)
        .then_some(date)
}

fn bounds(period: Period, date: NaiveDate, timezone: Tz) -> Option<Window> {
    let (start_date, next_date) = match period {
        Period::Day => (date, date.succ_opt()?),
        Period::Week => {
            let start =
                date.checked_sub_days(Days::new(date.weekday().num_days_from_monday().into()))?;
            (start, start.checked_add_days(Days::new(7))?)
        }
        Period::Month => {
            let start = date.with_day(1)?;
            (start, start.checked_add_months(chrono::Months::new(1))?)
        }
    };
    let end_date = next_date.pred_opt()?;
    if start_date.year() < 1 || end_date.year() > 9999 {
        return None;
    }
    Some(Window {
        period,
        start_date,
        end_date,
        start_ms: date_start(start_date, timezone)?.timestamp_millis(),
        end_ms: date_start(next_date, timezone)?.timestamp_millis(),
    })
}

fn date_start(date: NaiveDate, timezone: Tz) -> Option<DateTime<Tz>> {
    let midnight = date.and_hms_opt(0, 0, 0)?;
    // 重复午夜取较早时刻，跳过的午夜取缺口结束点，与宿主日历一致
    timezone
        .from_local_datetime(&midnight)
        .earliest()
        .or_else(|| GapInfo::new(&midnight, &timezone)?.end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        "2024-03-01T01:00:00Z".parse().unwrap()
    }

    #[test]
    fn defaults_to_deployment_date_and_uses_full_civil_periods() {
        let zone = chrono_tz::America::New_York;
        let month = Window::parse("period=month", zone, now()).unwrap();
        assert_eq!(month.start_date.to_string(), "2024-02-01");
        assert_eq!(month.end_date.to_string(), "2024-02-29");
        let week = Window::parse("period=week&date=2024-03-10", zone, now()).unwrap();
        assert_eq!(week.start_date.to_string(), "2024-03-04");
        assert_eq!(week.end_date.to_string(), "2024-03-10");
        assert_eq!(
            week.end_ms - week.start_ms,
            TimeDelta::hours(167).num_milliseconds()
        );
    }

    #[test]
    fn handles_skipped_dates_and_rejects_invalid_query_values() {
        let zone = chrono_tz::Pacific::Apia;
        let skipped = Window::parse("period=day&date=2011-12-30", zone, now()).unwrap();
        assert_eq!(skipped.start_ms, skipped.end_ms);
        for query in [
            "date=2024-02-10",
            "period=year",
            "period=day&date=2024-2-10",
            "period=day&date=2023-02-29",
            "period=day&date=",
            "period=day&period=week",
        ] {
            assert!(Window::parse(query, zone, now()).is_err(), "{query}");
        }
    }
}
