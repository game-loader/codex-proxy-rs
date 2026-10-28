//! 上游 Responses SSE 到 canonical 事件的解码；客户端字节原样透传，用量与费用旁路提取

use std::collections::BTreeMap;

use bytes::Bytes;
use gateway_core::error::{
    ClientVisibleUpstreamError, OpaqueUpstreamValue, ProviderError, ProviderErrorKind,
};
use gateway_core::event::{
    ContentItem, ContentKind, FinishReason, GatewayEvent, ProtocolWireEvent, ProviderEvent,
    ReasoningDelta, ResponseMeta, TextDelta, ToolCallDelta,
};
use gateway_core::metering::{
    CalculatedCostAmounts, CalculatedCostBreakdown, CalculatedCostRates, CurrencyCode, Decimal,
    ModelPriceOverride, Money, Usage,
};
use gateway_core::upstream::UpstreamSendState;
use gateway_protocol::openai::events::{TokenUsage, billable_usage_is_complete, extract_usage};
use gateway_protocol::openai::sse::{SseEvent, SseEventDecoder, SseFrame};
use serde_json::Value;

/// 透传给客户端的线协议；与 Provider 名称无关，表示 OpenAI Responses wire
pub const RESPONSES_PROTOCOL: &str = "openai";

const CONTENTS_PER_OUTPUT: u32 = 1_024;

/// 单次 attempt 的增量解码器
pub struct NewApiCanonicalDecoder {
    sse: SseEventDecoder,
    upstream_model: String,
    pricing: Option<ModelPriceOverride>,
    started: bool,
    completed: bool,
    response_id: Option<String>,
    content: BTreeMap<u32, ContentKind>,
}

impl NewApiCanonicalDecoder {
    #[must_use]
    pub fn new(upstream_model: impl Into<String>, pricing: Option<ModelPriceOverride>) -> Self {
        Self {
            sse: SseEventDecoder::default(),
            upstream_model: upstream_model.into(),
            pricing,
            started: false,
            completed: false,
            response_id: None,
            content: BTreeMap::new(),
        }
    }

    #[must_use]
    pub const fn is_completed(&self) -> bool {
        self.completed
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<ProviderEvent>, ProviderError> {
        let frames = self.sse.push_frames(chunk);
        self.decode_frames(frames)
    }

    /// 上游连接结束；未收到终态事件视为协议错误
    pub fn finish(&mut self) -> Result<Vec<ProviderEvent>, ProviderError> {
        let frames = self.sse.finish_frames();
        let events = self.decode_frames(frames)?;
        if !self.completed {
            return Err(protocol_error());
        }
        Ok(events)
    }

    fn decode_frames(
        &mut self,
        frames: Vec<SseFrame>,
    ) -> Result<Vec<ProviderEvent>, ProviderError> {
        let mut output = Vec::with_capacity(frames.len());
        for frame in frames {
            let (raw, events) = frame.into_parts();
            let raw = Bytes::from(raw);
            // 注释、心跳与无法观测的帧仍按原字节交付
            let Some(event) = events.into_iter().last() else {
                output.push(ProviderEvent::wire(wire_raw(raw)?));
                continue;
            };
            if event.data.trim() == "[DONE]" {
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(&event.data) else {
                output.push(ProviderEvent::wire(wire_raw(raw)?));
                continue;
            };
            let event_type = value
                .get("type")
                .and_then(Value::as_str)
                .or(event.event.as_deref())
                .unwrap_or_default()
                .to_owned();
            let mut canonical = Vec::new();
            let terminal = matches!(
                event_type.as_str(),
                "response.completed" | "response.incomplete"
            );
            match self.decode_event(&event_type, &value, &mut canonical) {
                Ok(()) => {}
                // 内容事件字段变体不打断客户端流，只放弃该帧的 canonical 提取
                Err(error) if !terminal && error.kind() == ProviderErrorKind::Protocol => {
                    canonical.clear();
                }
                Err(error) => return Err(error),
            }
            let wire = wire_json(&event, value, raw)?;
            output.push(if canonical.is_empty() {
                ProviderEvent::wire(wire)
            } else {
                ProviderEvent::canonical_with_wire(canonical, wire)
            });
        }
        Ok(output)
    }

    fn decode_event(
        &mut self,
        event_type: &str,
        value: &Value,
        output: &mut Vec<GatewayEvent>,
    ) -> Result<(), ProviderError> {
        if self.completed {
            return Err(protocol_error());
        }
        match event_type {
            "response.created" | "response.in_progress" => self.start(value, output),
            "response.output_item.added" => self.output_item_added(value, output),
            "response.content_part.added"
            | "response.reasoning_summary_part.added"
            | "response.reasoning_part.added" => self.content_part_added(value, output),
            "response.output_text.delta" | "response.refusal.delta" => {
                self.require_started()?;
                let index = event_content_index(value)?;
                self.ensure_content(index, ContentKind::Text, output)?;
                output.push(GatewayEvent::TextDelta(TextDelta {
                    content_index: index,
                    text: required_text(value, "delta")?,
                }));
                Ok(())
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                self.require_started()?;
                let index = content_index(
                    event_index(value, "output_index")?,
                    optional_index(value, "content_index")?
                        .or(optional_index(value, "summary_index")?)
                        .unwrap_or_default(),
                )?;
                self.ensure_content(index, ContentKind::Reasoning, output)?;
                output.push(GatewayEvent::ReasoningDelta(ReasoningDelta {
                    content_index: index,
                    text: required_text(value, "delta")?,
                }));
                Ok(())
            }
            "response.function_call_arguments.delta" | "response.custom_tool_call_input.delta" => {
                self.require_started()?;
                let index = content_index(event_index(value, "output_index")?, 0)?;
                self.ensure_content(index, ContentKind::ToolCall, output)?;
                let call_id = value
                    .get("call_id")
                    .or_else(|| value.get("item_id"))
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(protocol_error)?;
                output.push(GatewayEvent::ToolCallDelta(ToolCallDelta {
                    content_index: index,
                    call_id: call_id.to_owned(),
                    name: None,
                    arguments_delta: required_text(value, "delta")?,
                }));
                Ok(())
            }
            "response.completed" | "response.incomplete" => {
                self.complete(event_type, value, output)
            }
            "response.failed" | "error" => Err(upstream_event_error(value)),
            _ => Ok(()),
        }
    }

    fn start(
        &mut self,
        value: &Value,
        output: &mut Vec<GatewayEvent>,
    ) -> Result<(), ProviderError> {
        let response = value.get("response").ok_or_else(protocol_error)?;
        let response_id = required_text(response, "id")?;
        if self.started {
            return if self.response_id.as_deref() == Some(response_id.as_str()) {
                Ok(())
            } else {
                Err(protocol_error())
            };
        }
        let model = response_model(response, &self.upstream_model);
        self.response_id = Some(response_id.clone());
        self.started = true;
        output.push(GatewayEvent::Started(ResponseMeta::new(response_id, model)));
        Ok(())
    }

    fn output_item_added(
        &mut self,
        value: &Value,
        output: &mut Vec<GatewayEvent>,
    ) -> Result<(), ProviderError> {
        self.require_started()?;
        let item = value.get("item").ok_or_else(protocol_error)?;
        let index = content_index(event_index(value, "output_index")?, 0)?;
        match item.get("type").and_then(Value::as_str) {
            Some("function_call" | "custom_tool_call") => {
                self.add_content(index, ContentKind::ToolCall, output)?;
                let call_id = item
                    .get("call_id")
                    .or_else(|| item.get("id"))
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(protocol_error)?;
                output.push(GatewayEvent::ToolCallDelta(ToolCallDelta {
                    content_index: index,
                    call_id: call_id.to_owned(),
                    name: item
                        .get("name")
                        .and_then(Value::as_str)
                        .filter(|value| !value.is_empty())
                        .map(ToOwned::to_owned),
                    arguments_delta: String::new(),
                }));
                Ok(())
            }
            Some("reasoning") => self.add_content(index, ContentKind::Reasoning, output),
            _ => Ok(()),
        }
    }

    fn content_part_added(
        &mut self,
        value: &Value,
        output: &mut Vec<GatewayEvent>,
    ) -> Result<(), ProviderError> {
        self.require_started()?;
        let part = value
            .get("part")
            .or_else(|| value.get("summary_part"))
            .ok_or_else(protocol_error)?;
        let kind = match part.get("type").and_then(Value::as_str) {
            Some("output_text" | "refusal") => ContentKind::Text,
            Some("summary_text" | "reasoning_text") => ContentKind::Reasoning,
            _ => return Ok(()),
        };
        let part_index = optional_index(value, "content_index")?
            .or(optional_index(value, "summary_index")?)
            .unwrap_or_default();
        let index = content_index(event_index(value, "output_index")?, part_index)?;
        self.ensure_content(index, kind, output)
    }

    fn complete(
        &mut self,
        event_type: &str,
        value: &Value,
        output: &mut Vec<GatewayEvent>,
    ) -> Result<(), ProviderError> {
        self.require_started()?;
        let response = value.get("response").ok_or_else(protocol_error)?;
        let response_id = required_text(response, "id")?;
        if self.response_id.as_deref() != Some(response_id.as_str()) {
            return Err(protocol_error());
        }
        let usage = extract_usage(response);
        if let Some(usage) = usage {
            output.push(GatewayEvent::Usage(core_usage(usage)));
            // 只有完整的纯 Token 用量才按同步价目本地计费
            if billable_usage_is_complete(response, usage)
                && usage.image_input_tokens == 0
                && usage.image_output_tokens == 0
                && let Some(breakdown) = billing_breakdown(
                    usage.input_tokens,
                    usage.output_tokens,
                    usage.cached_input_tokens,
                    usage.cache_write_input_tokens,
                    self.pricing.as_ref(),
                )
            {
                output.push(GatewayEvent::CalculatedCost(breakdown.calculated_cost()));
            }
        }
        let incomplete = event_type == "response.incomplete"
            || response.get("status").and_then(Value::as_str) == Some("incomplete");
        let finish_reason = if incomplete {
            match response
                .pointer("/incomplete_details/reason")
                .and_then(Value::as_str)
            {
                Some("max_output_tokens" | "max_tokens") => FinishReason::Length,
                Some("content_filter") => FinishReason::ContentFilter,
                _ => FinishReason::Other,
            }
        } else if self
            .content
            .values()
            .any(|kind| *kind == ContentKind::ToolCall)
        {
            FinishReason::ToolCall
        } else {
            FinishReason::Stop
        };
        output.push(GatewayEvent::Completed(
            ResponseMeta::new(response_id, response_model(response, &self.upstream_model))
                .with_finish_reason(finish_reason),
        ));
        self.completed = true;
        Ok(())
    }

    fn add_content(
        &mut self,
        index: u32,
        kind: ContentKind,
        output: &mut Vec<GatewayEvent>,
    ) -> Result<(), ProviderError> {
        if self.content.insert(index, kind).is_some() {
            return Err(protocol_error());
        }
        output.push(GatewayEvent::ContentAdded(ContentItem::new(index, kind)));
        Ok(())
    }

    fn ensure_content(
        &mut self,
        index: u32,
        kind: ContentKind,
        output: &mut Vec<GatewayEvent>,
    ) -> Result<(), ProviderError> {
        match self.content.get(&index) {
            Some(current) if *current == kind => Ok(()),
            Some(_) => Err(protocol_error()),
            None => self.add_content(index, kind, output),
        }
    }

    fn require_started(&self) -> Result<(), ProviderError> {
        if self.started {
            Ok(())
        } else {
            Err(protocol_error())
        }
    }
}

/// 按 `standard` 档同步价或人工价计算费用；没有价目时不估价
#[must_use]
pub fn billing_breakdown(
    input_tokens: u64,
    output_tokens: u64,
    cached_input_tokens: u64,
    cache_write_input_tokens: u64,
    pricing: Option<&ModelPriceOverride>,
) -> Option<CalculatedCostBreakdown> {
    let pricing = pricing?;
    let rates = pricing.bands.get("standard")?;
    let uncached = input_tokens
        .checked_sub(cached_input_tokens)?
        .checked_sub(cache_write_input_tokens)?;
    let input = u128::from(uncached).checked_mul(rates.input.ticks_per_token())?;
    let cache_read =
        u128::from(cached_input_tokens).checked_mul(rates.cache_read.ticks_per_token())?;
    let cache_write =
        u128::from(cache_write_input_tokens).checked_mul(rates.cache_write.ticks_per_token())?;
    let output = u128::from(output_tokens).checked_mul(rates.output.ticks_per_token())?;
    let total = input
        .checked_add(cache_read)?
        .checked_add(cache_write)?
        .checked_add(output)?;
    CalculatedCostBreakdown::new(
        CalculatedCostAmounts::new(
            usd_money(input)?,
            usd_money(output)?,
            usd_money(cache_read)?,
            usd_money(cache_write)?,
            usd_money(total)?,
            usd_money(total)?,
        ),
        CalculatedCostRates::new(
            usd_per_million(rates.input.ticks_per_token())?,
            usd_per_million(rates.output.ticks_per_token())?,
            usd_per_million(rates.cache_read.ticks_per_token())?,
            usd_per_million(rates.cache_write.ticks_per_token())?,
        ),
        None,
        100,
    )
    .with_custom_multiplier(pricing.multiplier_bps)
}

fn usd_money(ticks: u128) -> Option<Money> {
    Some(Money::new(
        Decimal::from_scaled(ticks).ok()?,
        CurrencyCode::new("USD").ok()?,
    ))
}

fn usd_per_million(per_token_ticks: u128) -> Option<Money> {
    usd_money(per_token_ticks.checked_mul(1_000_000)?)
}

pub(crate) fn core_usage(usage: TokenUsage) -> Usage {
    let mut normalized = Usage::new();
    normalized.input_tokens = Some(usage.input_tokens);
    normalized.output_tokens = Some(usage.output_tokens);
    normalized.cached_input_tokens = Some(usage.cached_input_tokens);
    normalized.cache_write_input_tokens = Some(usage.cache_write_input_tokens);
    normalized.reasoning_output_tokens = Some(usage.reasoning_output_tokens);
    normalized.image_input_tokens = Some(usage.image_input_tokens);
    normalized.image_output_tokens = Some(usage.image_output_tokens);
    normalized.total_tokens = Some(usage.total_tokens);
    normalized
}

fn wire_raw(raw: Bytes) -> Result<ProtocolWireEvent, ProviderError> {
    ProtocolWireEvent::raw_sse(RESPONSES_PROTOCOL, raw).map_err(|_| protocol_error())
}

fn wire_json(
    event: &SseEvent,
    value: Value,
    raw: Bytes,
) -> Result<ProtocolWireEvent, ProviderError> {
    ProtocolWireEvent::json_with_raw_sse_metadata(
        RESPONSES_PROTOCOL,
        event.event.clone(),
        value,
        raw,
        event.id.clone(),
        event.retry,
    )
    .map_err(|_| protocol_error())
}

fn response_model(response: &Value, fallback: &str) -> String {
    response
        .get("model")
        .and_then(Value::as_str)
        .filter(|model| !model.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

fn required_text(value: &Value, field: &str) -> Result<String, ProviderError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(protocol_error)
}

fn optional_index(value: &Value, field: &str) -> Result<Option<u32>, ProviderError> {
    value
        .get(field)
        .map(|index| {
            index
                .as_u64()
                .and_then(|index| u32::try_from(index).ok())
                .ok_or_else(protocol_error)
        })
        .transpose()
}

fn event_index(value: &Value, field: &str) -> Result<u32, ProviderError> {
    optional_index(value, field)?.ok_or_else(protocol_error)
}

fn event_content_index(value: &Value) -> Result<u32, ProviderError> {
    content_index(
        event_index(value, "output_index")?,
        optional_index(value, "content_index")?.unwrap_or_default(),
    )
}

fn content_index(output_index: u32, part_index: u32) -> Result<u32, ProviderError> {
    if part_index >= CONTENTS_PER_OUTPUT {
        return Err(protocol_error());
    }
    output_index
        .checked_mul(CONTENTS_PER_OUTPUT)
        .and_then(|base| base.checked_add(part_index))
        .ok_or_else(protocol_error)
}

/// 上游 `response.failed` / `error` 事件；结构化文案按原样展示给客户端
pub(crate) fn upstream_event_error(value: &Value) -> ProviderError {
    let field = |name: &str| {
        value
            .pointer(&format!("/error/{name}"))
            .or_else(|| value.pointer(&format!("/response/error/{name}")))
            .or_else(|| value.get(name))
            .and_then(Value::as_str)
    };
    let code = field("code");
    let kind = match code {
        Some("invalid_request" | "invalid_request_error" | "invalid_prompt") => {
            ProviderErrorKind::InvalidRequest
        }
        Some("unauthorized" | "invalid_api_key" | "invalid_token") => {
            ProviderErrorKind::Unauthorized
        }
        Some("insufficient_quota") => ProviderErrorKind::QuotaExhausted,
        Some("rate_limit_exceeded") => ProviderErrorKind::RateLimited,
        _ => ProviderErrorKind::Unavailable,
    };
    let mut error = ProviderError::new(kind, UpstreamSendState::Sent);
    if let Some(code) = code {
        error = error.with_upstream_code(OpaqueUpstreamValue::new(code.to_owned()));
    }
    if let Some(message) = field("message") {
        error = error.with_client_visible_upstream_error(ClientVisibleUpstreamError::new(
            message,
            code.map(str::to_owned),
            field("type").map(str::to_owned),
        ));
    }
    error
}

pub(crate) fn protocol_error() -> ProviderError {
    ProviderError::new(ProviderErrorKind::Protocol, UpstreamSendState::Sent)
        .redact_sensitive_context("invalid upstream event")
}
