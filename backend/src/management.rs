//! 注册管理页与只读排行接口，隔离宿主统计错误

use chrono::Utc;
use chrono_tz::Tz;
use gateway_plugin_sdk::client::methods::RECONCILE;
use gateway_plugin_sdk::{
    ErrorCode, PluginFault,
    call::{
        management::{
            ManagementPage, ManagementRegistration, ManagementRequest, ManagementResource,
            ManagementResponse, ManagementRoute,
        },
        middleware::MiddlewareHeader,
    },
    client::{AuthorError, ComposedPlugin, PluginBuilder, TypedCall, TypedReply},
};
use serde::Serialize;
use serde_json::json;
use std::sync::Arc;

use crate::{
    AUTHOR_MANIFEST,
    calendar::{Period, Window},
    ranking::{self, ModelRow, RankingError, Row},
};

const ROUTE: &str = "ranking";
const JSON_TYPE: &str = "application/json";
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

pub(crate) fn plugin(timezone: Tz) -> Result<ComposedPlugin, AuthorError> {
    let maintenance = Arc::new(crate::archive::Maintenance::default());
    PluginBuilder::from_json(AUTHOR_MANIFEST)?
        .management(registration(), move |call| handle(call, timezone))?
        .on(RECONCILE, move |call| {
            let maintenance = Arc::clone(&maintenance);
            async move { maintenance.reconcile(call, timezone).await }
        })?
        .build()
}

fn registration() -> ManagementRegistration {
    ManagementRegistration {
        routes: vec![ManagementRoute {
            method: "GET".into(),
            path: ROUTE.into(),
            request_content_types: Vec::new(),
            response_content_types: vec![JSON_TYPE.into()],
        }],
        resources: ["web/index.html", "web/app.js", "web/app.css"]
            .into_iter()
            .map(|path| ManagementResource {
                path: path.into(),
                public: false,
            })
            .collect(),
        pages: vec![ManagementPage {
            id: "key-usage-ranking".into(),
            title: "Key 用量排行".into(),
            description: None,
            entry: "web/index.html".into(),
            icon: None,
        }],
        callbacks: Vec::new(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    period: Period,
    start_date: String,
    end_date: String,
    timezone: &'static str,
    #[serde(flatten)]
    data: ReportData,
}

#[derive(Serialize)]
#[serde(untagged)]
enum ReportData {
    Keys { rows: Vec<Row> },
    Models { models: Vec<ModelRow> },
}

type Reply = Result<TypedReply<ManagementResponse>, PluginFault>;

async fn handle(call: TypedCall<ManagementRequest>, timezone: Tz) -> Reply {
    match (call.request.method.as_str(), call.request.path.as_str()) {
        ("GET", ROUTE) => (),
        _ => return error(404, "not_found", "未找到插件管理接口"),
    }
    if !call.payload.is_empty() {
        return error(400, "invalid_query", "排行查询不接受请求正文");
    }
    let now = Utc::now();
    let (window, key_id) = match Window::parse(&call.request.query, timezone, now) {
        Ok(query) => query,
        Err(message) => return error(400, "invalid_query", message),
    };
    let data = if let Some(key_id) = key_id {
        crate::archive::load_models(&call.host, &window, timezone, now, &key_id)
            .await
            .map(|models| ReportData::Models { models })
    } else {
        match window.period {
            Period::Month => crate::archive::load_month(&call.host, &window, timezone, now).await,
            Period::Day | Period::Week => {
                ranking::load(&call.host, &window, now.timestamp_millis()).await
            }
        }
        .map(|rows| ReportData::Keys { rows })
    };
    let data = match data {
        Ok(data) => data,
        Err(cause) => return ranking_error(cause),
    };
    let body = encode(&Report {
        period: window.period,
        start_date: window.start_date.to_string(),
        end_date: window.end_date.to_string(),
        timezone: timezone.name(),
        data,
    })?;
    if body.len() > MAX_RESPONSE_BYTES {
        return ranking_error(RankingError::TooLarge);
    }
    Ok(reply(200, body))
}

fn ranking_error(cause: RankingError) -> Reply {
    match cause {
        RankingError::HistoryUnavailable => error(
            422,
            "history_unavailable",
            "所选时段缺少完整的宿主用量记录或归档",
        ),
        RankingError::ProjectionUnavailable => {
            error(503, "statistics_unavailable", "宿主用量统计暂不可用")
        }
        RankingError::KeyNamesUnavailable => {
            error(503, "key_names_unavailable", "无法读取 Key 名称")
        }
        RankingError::InvalidStatistics => {
            error(502, "invalid_statistics", "宿主未返回完整用量统计")
        }
        RankingError::TooLarge => error(422, "ranking_too_large", "排行结果过多，请选择较短时段"),
        RankingError::Timeout => error(504, "statistics_timeout", "排行查询超时"),
    }
}

fn error(status: u16, code: &str, message: &str) -> Reply {
    Ok(reply(
        status,
        encode(&json!({"error": {"code": code, "message": message}}))?,
    ))
}

fn encode(value: &impl Serialize) -> Result<Vec<u8>, PluginFault> {
    serde_json::to_vec(value).map_err(|_| PluginFault::new(ErrorCode::Fault, "无法编码排行结果"))
}

fn reply(status: u16, payload: Vec<u8>) -> TypedReply<ManagementResponse> {
    TypedReply::new(ManagementResponse {
        status,
        content_type: JSON_TYPE.into(),
        headers: vec![MiddlewareHeader {
            name: "cache-control".into(),
            value: b"no-store".to_vec(),
        }],
    })
    .with_payload(payload)
}

#[cfg(test)]
mod tests;
