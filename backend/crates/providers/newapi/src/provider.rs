//! new-api 兼容上游的 `gateway-core` Provider；只处理 OpenAI Responses HTTP/SSE 生成请求

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use futures::StreamExt;
use gateway_core::account::AccountFeedbackStats;
use gateway_core::engine::AttemptContext;
use gateway_core::engine::middleware::MiddlewareHeader;
use gateway_core::engine::provider::{
    EventStream, Provider, ProviderCallMetadata, ProviderRequest, ProviderSelectionObservation,
    ProviderStream,
};
use gateway_core::error::{ProviderError, ProviderErrorKind};
use gateway_core::operation::{Feature, GenerateRequest, Operation, OperationKind};
use gateway_core::routing::{
    ModelCapabilities, ProviderCatalogGeneration, ProviderKind, ProviderModelCapabilities,
    SupportLevel, UpstreamModelId,
};
use gateway_core::upstream::{UpstreamSendState, UpstreamTransport};
use serde_json::Value;

use crate::canonical::{NewApiCanonicalDecoder, RESPONSES_PROTOCOL};
use crate::catalog::NewApiCatalog;
use crate::selector::{NewApiSelector, SelectedAccount};

pub const PROVIDER_NAME: &str = "newapi";
const HTTP_SSE_TRANSPORT: &str = "http_sse";

pub struct NewApiProvider {
    selector: NewApiSelector,
    catalog: Arc<NewApiCatalog>,
    account_feedback: Arc<AccountFeedbackStats>,
}

impl NewApiProvider {
    pub fn new(
        selector: NewApiSelector,
        catalog: Arc<NewApiCatalog>,
        account_feedback: Arc<AccountFeedbackStats>,
    ) -> Self {
        Self {
            selector,
            catalog,
            account_feedback,
        }
    }
}

#[async_trait]
impl Provider for NewApiProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn catalog_generation(&self) -> ProviderCatalogGeneration {
        self.catalog.generation()
    }

    // 目录只做发现；未列出的模型仍交给上游判断
    fn model_catalog_is_exhaustive(&self) -> bool {
        false
    }

    async fn query_model_capabilities(
        &self,
    ) -> Result<Vec<ProviderModelCapabilities>, ProviderError> {
        let snapshot = self
            .catalog
            .synchronize()
            .await
            .map_err(|_| not_sent(ProviderErrorKind::Unavailable))?;
        snapshot
            .models
            .into_iter()
            .map(|(model, accounts)| {
                let model = UpstreamModelId::new(model)
                    .map_err(|_| not_sent(ProviderErrorKind::Protocol))?;
                Ok(ProviderModelCapabilities::new(model, model_capabilities())
                    .with_catalog_accounts(accounts))
            })
            .collect()
    }

    async fn execute(
        self: Arc<Self>,
        request: ProviderRequest,
        context: AttemptContext,
    ) -> Result<ProviderStream, ProviderError> {
        if request.candidate().provider().as_str() != PROVIDER_NAME {
            return Err(not_sent(ProviderErrorKind::InvalidRequest));
        }
        if context.cancellation().is_cancelled() {
            return Err(not_sent(ProviderErrorKind::Cancelled));
        }
        if context.deadline().is_elapsed() {
            return Err(not_sent(ProviderErrorKind::Timeout));
        }
        let Operation::Generate(generate) = request.operation() else {
            return Err(not_sent(ProviderErrorKind::Unsupported));
        };
        if generate.protocol_payload().protocol() != RESPONSES_PROTOCOL {
            return Err(not_sent(ProviderErrorKind::Unsupported));
        }
        let upstream_model = request
            .candidate()
            .upstream_model()
            .cloned()
            .ok_or_else(|| not_sent(ProviderErrorKind::InvalidRequest))?;
        let selection_started_at = Instant::now();
        let selected = self.selector.select(&upstream_model, &context).await?;
        let wait_ms = u64::try_from(selection_started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
        let provider_kind =
            ProviderKind::new(PROVIDER_NAME).map_err(|_| not_sent(ProviderErrorKind::Protocol))?;
        let account_id = selected.account_id.clone();
        let middleware_context = context.clone();
        let provider = Arc::clone(&self);
        middleware_context
            .execute_middleware(
                Operation::Generate(generate.clone()),
                provider_kind,
                Some(upstream_model.as_str().to_owned()),
                account_id,
                Box::new(move |operation, headers| {
                    Box::pin(async move {
                        provider.prepare_stream(
                            operation,
                            headers,
                            selected,
                            upstream_model,
                            wait_ms,
                            context,
                        )
                    })
                }),
            )
            .await
    }
}

impl NewApiProvider {
    fn prepare_stream(
        &self,
        operation: Operation,
        headers: Vec<MiddlewareHeader>,
        selected: SelectedAccount,
        upstream_model: UpstreamModelId,
        wait_ms: u64,
        context: AttemptContext,
    ) -> Result<ProviderStream, ProviderError> {
        let Operation::Generate(generate) = operation else {
            return Err(not_sent(ProviderErrorKind::Unsupported));
        };
        let body = request_body(&generate, upstream_model.as_str())?;
        let pricing = context
            .pricing()
            .get(PROVIDER_NAME)
            .and_then(|models| models.get(upstream_model.as_str()))
            .cloned();
        let SelectedAccount {
            account_id,
            credential,
            outbound_proxy,
            account_snapshot,
            capacity,
            lease,
        } = selected;
        let client = crate::http::client(outbound_proxy.as_ref())
            .map_err(|_| not_sent(ProviderErrorKind::ProviderInfrastructureUnavailable))?;
        let metadata = ProviderCallMetadata::new(
            ProviderKind::new(PROVIDER_NAME).map_err(|_| not_sent(ProviderErrorKind::Protocol))?,
            upstream_model.clone(),
            account_id,
            UpstreamTransport::new(HTTP_SSE_TRANSPORT)
                .map_err(|_| not_sent(ProviderErrorKind::Protocol))?,
        )
        .with_account_snapshot(Some(account_snapshot))
        .with_selection_observation(ProviderSelectionObservation::new(wait_ms, capacity));
        let mut builder = client
            .post(credential.endpoint("responses"))
            .bearer_auth(&credential.api_key)
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(
                serde_json::to_vec(&body)
                    .map_err(|_| not_sent(ProviderErrorKind::InvalidRequest))?,
            );
        // 中间件追加的业务头原样转交；认证与传输头由本 Provider 持有
        for header in headers {
            let (name, value) = header.into_parts();
            if gateway_protocol::openai::is_transport_managed_request_header(&name)
                || name.eq_ignore_ascii_case("authorization")
            {
                continue;
            }
            builder = builder.header(name, value.to_vec());
        }
        if let Some(remaining) = context.deadline().remaining() {
            builder = builder.timeout(remaining);
        }
        let cancellation = context.cancellation().clone();
        let model = upstream_model.as_str().to_owned();
        // 冷流：只有被 poll 后才真正发送
        let events: EventStream = Box::pin(async_stream(builder, cancellation, model, pricing));
        Ok(ProviderStream::new(metadata, events, lease)
            .with_account_feedback(Arc::clone(&self.account_feedback)))
    }
}

fn async_stream(
    builder: reqwest::RequestBuilder,
    cancellation: gateway_core::lifecycle::CancellationToken,
    model: String,
    pricing: Option<gateway_core::metering::ModelPriceOverride>,
) -> impl futures::Stream<Item = Result<gateway_core::event::ProviderEvent, ProviderError>>
+ Send
+ 'static {
    futures::stream::once(async move {
        let response = tokio::select! {
            biased;
            () = cancellation.cancelled() => {
                return futures::stream::iter(vec![Err(not_sent(ProviderErrorKind::Cancelled))]).boxed();
            }
            response = builder.send() => response,
        };
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                return futures::stream::iter(vec![Err(crate::http::send_error(error))]).boxed();
            }
        };
        let status = response.status();
        if !status.is_success() {
            let retry_after = crate::http::retry_after(response.headers());
            let body = crate::http::bounded_body(response, crate::http::MAX_ERROR_BODY_BYTES).await;
            return futures::stream::iter(vec![Err(crate::http::status_error(
                status,
                retry_after,
                &body,
            ))])
            .boxed();
        }
        let decoder = NewApiCanonicalDecoder::new(model, pricing);
        decode_body(response.bytes_stream(), decoder, cancellation).boxed()
    })
    .flatten()
}

fn decode_body(
    body: impl futures::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send + Unpin + 'static,
    decoder: NewApiCanonicalDecoder,
    cancellation: gateway_core::lifecycle::CancellationToken,
) -> impl futures::Stream<Item = Result<gateway_core::event::ProviderEvent, ProviderError>>
+ Send
+ 'static {
    struct State<S> {
        body: S,
        decoder: NewApiCanonicalDecoder,
        cancellation: gateway_core::lifecycle::CancellationToken,
        done: bool,
    }
    futures::stream::unfold(
        State {
            body,
            decoder,
            cancellation,
            done: false,
        },
        |mut state| async move {
            if state.done {
                return None;
            }
            let chunk = tokio::select! {
                biased;
                () = state.cancellation.cancelled() => {
                    state.done = true;
                    return Some((vec![Err(ProviderError::new(ProviderErrorKind::Cancelled, UpstreamSendState::Sent))], state));
                }
                chunk = state.body.next() => chunk,
            };
            let batch = match chunk {
                Some(Ok(chunk)) => state.decoder.push(&chunk),
                Some(Err(error)) => {
                    state.done = true;
                    return Some((vec![Err(crate::http::stream_error(error))], state));
                }
                None => {
                    state.done = true;
                    state.decoder.finish()
                }
            };
            let items = match batch {
                Ok(events) => events.into_iter().map(Ok).collect(),
                Err(error) => {
                    state.done = true;
                    vec![Err(error)]
                }
            };
            // 已完成后不再继续读取上游，避免终态之后的帧
            if state.decoder.is_completed() {
                state.done = true;
            }
            Some((items, state))
        },
    )
    .flat_map(futures::stream::iter)
}

/// 透传客户端正文，只替换上游模型并强制流式
fn request_body(generate: &GenerateRequest, upstream_model: &str) -> Result<Value, ProviderError> {
    let mut body = generate.protocol_payload().body().clone();
    body.insert("model".to_owned(), Value::String(upstream_model.to_owned()));
    body.insert("stream".to_owned(), Value::Bool(true));
    Ok(Value::Object(body))
}

fn model_capabilities() -> ModelCapabilities {
    ModelCapabilities::new(BTreeSet::from([OperationKind::Generate]), None)
        .with_upstream_feature_validation()
        .with_feature(Feature::Tools, SupportLevel::Unknown)
        .with_feature(Feature::Reasoning, SupportLevel::Unknown)
        .with_feature(Feature::Vision, SupportLevel::Unknown)
        .with_feature(Feature::JsonSchema, SupportLevel::Unknown)
        .with_feature(Feature::NativeContinuation, SupportLevel::Unsupported)
}

fn not_sent(kind: ProviderErrorKind) -> ProviderError {
    ProviderError::new(kind, UpstreamSendState::NotSent)
}
