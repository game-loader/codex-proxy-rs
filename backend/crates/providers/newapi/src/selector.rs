//! 账号选择与调度租约；只按范围、模型政策、权重和并发选号，不做会话亲和

use std::sync::Arc;
use std::time::SystemTime;

use gateway_core::account::{
    AccountCandidate, AccountCapacitySnapshot, AccountEligibilityPolicy, AccountFeedbackStats,
    AccountHistorySnapshot, AccountSelectionContext, AccountSelector, ProviderAccount,
    ProviderAccountId, ProviderAccountStore,
};
use gateway_core::engine::AttemptContext;
use gateway_core::engine::policy::AccountPolicyError;
use gateway_core::error::{ErrorSource, ProviderError, ProviderErrorKind};
use gateway_core::provider_ports::{
    ProviderConcurrencyPool, ProviderLeaseAcquisition, ProviderLeaseGuard, ProviderLeasePort,
    ProviderLeaseRequest, ProviderSchedulingLeaseRequest,
};
use gateway_core::routing::{ProviderKind, UpstreamModelId};
use gateway_core::upstream::UpstreamSendState;

use crate::credential::{AUTHENTICATION_KIND, NewApiCredential};

/// 已取得租约的账号；租约随 Provider stream 释放
pub struct SelectedAccount {
    pub account_id: ProviderAccountId,
    pub credential: NewApiCredential,
    pub outbound_proxy: Option<gateway_core::account::OutboundProxy>,
    pub account_snapshot: AccountHistorySnapshot,
    pub capacity: Option<AccountCapacitySnapshot>,
    pub lease: Box<dyn ProviderLeaseGuard>,
}

pub struct NewApiSelector {
    provider_kind: ProviderKind,
    accounts: Arc<dyn ProviderAccountStore>,
    leases: Arc<dyn ProviderLeasePort>,
    account_feedback: Arc<AccountFeedbackStats>,
}

impl NewApiSelector {
    pub fn new(
        provider_kind: ProviderKind,
        accounts: Arc<dyn ProviderAccountStore>,
        leases: Arc<dyn ProviderLeasePort>,
        account_feedback: Arc<AccountFeedbackStats>,
    ) -> Self {
        Self {
            provider_kind,
            accounts,
            leases,
            account_feedback,
        }
    }

    pub async fn select(
        &self,
        model: &UpstreamModelId,
        context: &AttemptContext,
    ) -> Result<SelectedAccount, ProviderError> {
        let diagnostic = context.is_diagnostic_required_account();
        // 诊断请求不带账号范围；常规请求必须由 Core 冻结范围
        let scope = match (diagnostic, context.account_scope()) {
            (true, scope) => scope.cloned(),
            (false, Some(scope)) => Some(Arc::clone(scope)),
            (false, None) => return Err(not_sent(ProviderErrorKind::NoEligibleAccount)),
        };
        'refresh: loop {
            let mut accounts = self
                .accounts
                .list_for_provider(&self.provider_kind)
                .await
                .map_err(infrastructure)?;
            if diagnostic
                && let Some(required) = context.required_account()
                && !accounts.iter().any(|account| account.id() == required)
                && let Some(account) = self
                    .accounts
                    .get_account(required)
                    .await
                    .map_err(infrastructure)?
                && account.provider() == &self.provider_kind
            {
                accounts.push(account);
            }
            let accounts: Vec<ProviderAccount> = accounts
                .into_iter()
                .filter(|account| account.authentication_kind() == AUTHENTICATION_KIND)
                .filter(|account| {
                    diagnostic
                        || scope
                            .as_ref()
                            .is_some_and(|scope| scope.allows_model(account.id(), model.as_str()))
                })
                .filter(|account| {
                    context
                        .required_account()
                        .is_none_or(|required| account.id() == required)
                })
                .collect();
            if accounts.is_empty() {
                return Err(not_sent(ProviderErrorKind::NoEligibleAccount));
            }
            let ids: Vec<_> = accounts
                .iter()
                .map(|account| account.id().clone())
                .collect();
            let scheduling = self
                .leases
                .load_state(
                    context.client_api_key_ref(),
                    &self.provider_kind,
                    &ids,
                    ProviderConcurrencyPool::Shared,
                )
                .await
                .map_err(infrastructure)?;
            let candidates = accounts
                .into_iter()
                .map(|account| {
                    let (failure_rate, latency) = self
                        .account_feedback
                        .scheduling_signals(&self.provider_kind, account.id());
                    let signals = scheduling
                        .signals()
                        .get(account.id())
                        .cloned()
                        .ok_or_else(|| {
                            not_sent(ProviderErrorKind::ProviderInfrastructureUnavailable)
                        })?
                        .with_runtime_health(failure_rate, latency);
                    Ok(AccountCandidate { account, signals })
                })
                .collect::<Result<Vec<_>, ProviderError>>()?;
            let mut selection_context = AccountSelectionContext {
                policy: context.account_selection_policy(),
                now: SystemTime::now(),
                excluded_accounts: context.excluded_accounts().clone(),
                busy_accounts: Default::default(),
                preferred_account: context.required_account().cloned(),
                preferred_account_overrides_weight: false,
                round_robin_cursor: scheduling.round_robin_cursor(),
                eligibility: if diagnostic {
                    AccountEligibilityPolicy::BypassForDiagnostic
                } else {
                    AccountEligibilityPolicy::Enforce
                },
                account_scope: scope.clone().filter(|_| !diagnostic),
                reserved_concurrency: 0,
            };
            let mut retry_after = None;
            let mut capacity_denied = false;
            loop {
                selection_context.excluded_accounts = context.excluded_accounts().clone();
                let selection = match context.request_policy_context() {
                    Some(policy) => policy
                        .select_account(
                            context.attempt_index(),
                            &self.provider_kind,
                            Some(model.as_str()),
                            &candidates,
                            &mut selection_context,
                        )
                        .await
                        .map_err(|error| match error {
                            AccountPolicyError::StaleCandidate => None,
                            AccountPolicyError::Filtered | AccountPolicyError::Rejected => {
                                Some(not_sent(ProviderErrorKind::RequestPolicyDenied))
                            }
                            AccountPolicyError::Fault => {
                                Some(not_sent(ProviderErrorKind::Unavailable))
                            }
                        }),
                    None => Ok(AccountSelector.select(&candidates, &selection_context)),
                };
                let selection = match selection {
                    Ok(selection) => selection,
                    Err(None) => continue 'refresh,
                    Err(Some(error)) => return Err(error),
                };
                let Some(selection) = selection else {
                    break;
                };
                let capacity = AccountSelector.capacity_snapshot(&candidates, &selection_context);
                let selected = selection.candidate();
                let selected_id = selected.account.id().clone();
                let revision = selected.account.revision();
                let lease = self
                    .leases
                    .try_acquire(ProviderLeaseRequest::Scheduling(
                        ProviderSchedulingLeaseRequest::new(
                            self.provider_kind.clone(),
                            selected_id.clone(),
                            revision,
                            selected.account.effective_concurrency(
                                context
                                    .account_selection_policy()
                                    .max_concurrent_per_account(),
                            ),
                            context.deadline(),
                        )
                        .with_metadata(context.lease_metadata())
                        .with_cancellation(context.cancellation().clone()),
                    ))
                    .await
                    .map_err(infrastructure)?;
                let lease = match lease {
                    ProviderLeaseAcquisition::Acquired(lease) => lease,
                    ProviderLeaseAcquisition::Busy {
                        retry_after: candidate_retry,
                    } => {
                        capacity_denied = true;
                        retry_after = match (retry_after, candidate_retry) {
                            (Some(current), Some(next)) => Some(std::cmp::min(current, next)),
                            (current, next) => current.or(next),
                        };
                        selection_context.busy_accounts.insert(selected_id);
                        continue;
                    }
                };
                let loaded = self
                    .accounts
                    .load_credential(&selected_id, revision)
                    .await
                    .map_err(infrastructure)?;
                let credential =
                    NewApiCredential::from_document(&loaded.credential.clone().into_inner())
                        .map_err(|_| not_sent(ProviderErrorKind::Unauthorized))?;
                return Ok(SelectedAccount {
                    account_id: selected_id,
                    credential,
                    outbound_proxy: loaded.account.outbound_proxy().cloned(),
                    account_snapshot: AccountHistorySnapshot::from_account(&loaded.account),
                    capacity: capacity.map(AccountCapacitySnapshot::with_acquired_request),
                    lease,
                });
            }
            let mut error = not_sent(if capacity_denied {
                ProviderErrorKind::AccountCapacityUnavailable
            } else {
                ProviderErrorKind::NoEligibleAccount
            });
            if let Some(retry_after) = retry_after {
                error = error.with_retry_after(retry_after);
            }
            return Err(error);
        }
    }
}

fn not_sent(kind: ProviderErrorKind) -> ProviderError {
    ProviderError::new(kind, UpstreamSendState::NotSent)
}

fn infrastructure(source: impl std::error::Error + Send + Sync + 'static) -> ProviderError {
    not_sent(ProviderErrorKind::ProviderInfrastructureUnavailable)
        .with_source(ErrorSource::new(source))
}
