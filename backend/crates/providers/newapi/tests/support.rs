//! 测试辅助：只读账号仓库，以及目录对账不会调用的端口占位实现

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::future::BoxFuture;
use gateway_core::account::{
    AccountStateChange, CredentialCasOutcome, CredentialCasUpdate, CredentialRevision,
    LoadedCredential, NewProviderAccount, OpaqueProviderData, PlaintextCredential, ProviderAccount,
    ProviderAccountId, ProviderAccountStore, ProviderAccountUpdate, ProviderRefreshQuery,
    QuotaAccessChange, QuotaObservation, QuotaObservationTouch, QuotaWriteOutcome,
};
use gateway_core::diagnostics::{OperationalDiagnostics, OperationalFailure};
use gateway_core::error::{StoreError, StoreErrorKind};
use gateway_core::identity::ProviderKind;
use gateway_core::policy::ClientApiKeyId;
use gateway_core::provider_ports::{
    NewOAuthPendingFlow, OAuthPendingBinding, OAuthPendingClaimOutcome, OAuthPendingConsumeOutcome,
    OAuthPendingFlowPort, OAuthPendingPutOutcome, OAuthPendingReleaseOutcome,
    ProviderArtifactProfile, ProviderArtifactProfileCachePort, ProviderCatalogCacheKey,
    ProviderCatalogCachePort, ProviderConcurrencyPool, ProviderCooldown, ProviderCooldownPort,
    ProviderCooldownScope, ProviderCredentialState, ProviderCredentialStatePort,
    ProviderLeaseAcquisition, ProviderLeasePort, ProviderLeaseRequest, ProviderRefreshPolicy,
    ProviderRuntimePolicyPort, ProviderSchedulingState, ProviderScopedCooldown,
    ProviderSessionAffinityKey, ProviderSessionAffinityPort, ProviderSessionAlias,
    ProviderSessionBinding, ProviderSessionExclusionPort, ProviderSessionExclusions,
    ProviderStoreError, ProviderStoreErrorKind, ProviderStorePorts,
};
use provider_newapi::{AUTHENTICATION_KIND, NewApiCredential, PROVIDER_NAME};

/// 由单个只读账号组成的 Provider 端口集合
pub fn ports_with_account(id: &str, base_url: &str) -> ProviderStorePorts {
    ProviderStorePorts::new(
        Arc::new(StaticAccounts::single(id, base_url)),
        Arc::new(Unused),
        Arc::new(Unused),
        Arc::new(Unused),
        Arc::new(Unused),
        Arc::new(Unused),
        Arc::new(Unused),
        Arc::new(Unused),
        Arc::new(Unused),
        Arc::new(Unused),
        Arc::new(Unused),
    )
}

/// 目录对账只调用列表与凭据读取，写入一律视为测试越界
struct StaticAccounts {
    account: ProviderAccount,
    credential: PlaintextCredential,
}

impl StaticAccounts {
    fn single(id: &str, base_url: &str) -> Self {
        let account = ProviderAccount::new(
            ProviderAccountId::new(id).unwrap(),
            ProviderKind::new(PROVIDER_NAME).unwrap(),
            id.to_owned(),
            None,
            AUTHENTICATION_KIND.to_owned(),
            CredentialRevision::new(1).unwrap(),
            None,
        );
        let credential = NewApiCredential::new(base_url, "sk-test").unwrap();
        Self {
            account,
            credential: PlaintextCredential::new(credential.into_document()),
        }
    }

    fn loaded(&self, id: &ProviderAccountId) -> Result<LoadedCredential, StoreError> {
        if id != self.account.id() {
            return Err(StoreError::new(StoreErrorKind::InvalidData));
        }
        Ok(LoadedCredential {
            account: self.account.clone(),
            credential: self.credential.clone(),
        })
    }
}

fn store_unused() -> StoreError {
    StoreError::new(StoreErrorKind::Unavailable)
}

#[async_trait]
impl ProviderAccountStore for StaticAccounts {
    async fn create_account(&self, _: NewProviderAccount) -> Result<(), StoreError> {
        Err(store_unused())
    }

    async fn get_account(
        &self,
        account: &ProviderAccountId,
    ) -> Result<Option<ProviderAccount>, StoreError> {
        Ok((account == self.account.id()).then(|| self.account.clone()))
    }

    async fn list_accounts(&self) -> Result<Vec<ProviderAccount>, StoreError> {
        Ok(vec![self.account.clone()])
    }

    async fn list_for_provider(
        &self,
        provider: &ProviderKind,
    ) -> Result<Vec<ProviderAccount>, StoreError> {
        Ok(if provider == self.account.provider() {
            vec![self.account.clone()]
        } else {
            Vec::new()
        })
    }

    async fn list_refresh_candidates(
        &self,
        _: ProviderRefreshQuery,
    ) -> Result<Vec<LoadedCredential>, StoreError> {
        Ok(Vec::new())
    }

    async fn load_credential(
        &self,
        account: &ProviderAccountId,
        _: CredentialRevision,
    ) -> Result<LoadedCredential, StoreError> {
        self.loaded(account)
    }

    async fn load_current_credential(
        &self,
        account: &ProviderAccountId,
    ) -> Result<LoadedCredential, StoreError> {
        self.loaded(account)
    }

    async fn compare_and_swap_credential(
        &self,
        _: CredentialCasUpdate,
    ) -> Result<CredentialCasOutcome, StoreError> {
        Err(store_unused())
    }

    async fn get_quotas(
        &self,
        _: &[ProviderAccountId],
    ) -> Result<Vec<QuotaObservation>, StoreError> {
        Ok(Vec::new())
    }

    async fn compare_and_swap_quota(
        &self,
        _: QuotaObservation,
    ) -> Result<QuotaWriteOutcome, StoreError> {
        Err(store_unused())
    }

    async fn touch_quota_observation(
        &self,
        _: QuotaObservationTouch,
    ) -> Result<QuotaWriteOutcome, StoreError> {
        Err(store_unused())
    }

    async fn apply_quota_access(
        &self,
        _: QuotaAccessChange,
    ) -> Result<QuotaWriteOutcome, StoreError> {
        Err(store_unused())
    }

    async fn apply_state_change(&self, _: AccountStateChange) -> Result<(), StoreError> {
        Err(store_unused())
    }

    async fn update_account(&self, _: ProviderAccountUpdate) -> Result<(), StoreError> {
        Err(store_unused())
    }

    async fn set_enabled(&self, _: &ProviderAccountId, _: bool) -> Result<(), StoreError> {
        Err(store_unused())
    }

    async fn delete_account(&self, _: &ProviderAccountId) -> Result<(), StoreError> {
        Err(store_unused())
    }
}

/// 目录对账路径不会调用的端口；被调用即返回不可用
struct Unused;

fn port_unused() -> ProviderStoreError {
    ProviderStoreError::new(ProviderStoreErrorKind::Unavailable, "unused test port")
}

fn unused_future<'a, T: Send + 'a>() -> BoxFuture<'a, Result<T, ProviderStoreError>> {
    Box::pin(async { Err(port_unused()) })
}

impl ProviderLeasePort for Unused {
    fn load_state<'a>(
        &'a self,
        _: &'a ClientApiKeyId,
        _: &'a ProviderKind,
        _: &'a [ProviderAccountId],
        _: ProviderConcurrencyPool,
    ) -> BoxFuture<'a, Result<ProviderSchedulingState, ProviderStoreError>> {
        unused_future()
    }

    fn try_acquire(
        &self,
        _: ProviderLeaseRequest,
    ) -> BoxFuture<'_, Result<ProviderLeaseAcquisition, ProviderStoreError>> {
        unused_future()
    }
}

impl ProviderSessionAffinityPort for Unused {
    fn load<'a>(
        &'a self,
        _: &'a ProviderKind,
        _: &'a ProviderSessionAffinityKey,
    ) -> BoxFuture<'a, Result<Option<ProviderSessionBinding>, ProviderStoreError>> {
        unused_future()
    }

    fn compare_and_bind<'a>(
        &'a self,
        _: &'a ProviderKind,
        _: &'a ProviderSessionAffinityKey,
        _: Option<&'a ProviderSessionBinding>,
        _: &'a ProviderAccountId,
        _: Duration,
    ) -> BoxFuture<'a, Result<Option<ProviderSessionBinding>, ProviderStoreError>> {
        unused_future()
    }

    fn load_alias<'a>(
        &'a self,
        _: &'a ProviderKind,
        _: &'a ProviderSessionAffinityKey,
    ) -> BoxFuture<'a, Result<Option<ProviderSessionAlias>, ProviderStoreError>> {
        unused_future()
    }

    fn bind_alias<'a>(
        &'a self,
        _: &'a ProviderKind,
        _: &'a ProviderSessionAffinityKey,
        _: &'a ProviderSessionAlias,
        _: Duration,
    ) -> BoxFuture<'a, Result<bool, ProviderStoreError>> {
        unused_future()
    }
}

impl ProviderSessionExclusionPort for Unused {
    fn load<'a>(
        &'a self,
        _: &'a ProviderKind,
        _: &'a ProviderSessionAffinityKey,
    ) -> BoxFuture<'a, Result<Option<ProviderSessionExclusions>, ProviderStoreError>> {
        unused_future()
    }

    fn record_failure<'a>(
        &'a self,
        _: &'a ProviderKind,
        _: &'a ProviderSessionAffinityKey,
        _: &'a ProviderAccountId,
        _: Duration,
    ) -> BoxFuture<'a, Result<ProviderSessionExclusions, ProviderStoreError>> {
        unused_future()
    }

    fn clear<'a>(
        &'a self,
        _: &'a ProviderKind,
        _: &'a ProviderSessionAffinityKey,
        _: &'a str,
    ) -> BoxFuture<'a, Result<bool, ProviderStoreError>> {
        unused_future()
    }
}

impl ProviderCatalogCachePort for Unused {
    fn replace<'a>(
        &'a self,
        _: &'a ProviderCatalogCacheKey,
        _: &'a OpaqueProviderData,
        _: Duration,
    ) -> BoxFuture<'a, Result<(), ProviderStoreError>> {
        unused_future()
    }

    fn read<'a>(
        &'a self,
        _: &'a ProviderCatalogCacheKey,
    ) -> BoxFuture<'a, Result<Option<OpaqueProviderData>, ProviderStoreError>> {
        unused_future()
    }
}

impl ProviderArtifactProfileCachePort for Unused {
    fn replace_if_newer(
        &self,
        _: ProviderArtifactProfile,
        _: Duration,
    ) -> BoxFuture<'_, Result<bool, ProviderStoreError>> {
        unused_future()
    }

    fn read<'a>(
        &'a self,
        _: &'a ProviderKind,
        _: &'a str,
    ) -> BoxFuture<'a, Result<Option<ProviderArtifactProfile>, ProviderStoreError>> {
        unused_future()
    }
}

impl ProviderCredentialStatePort for Unused {
    fn replace(&self, _: ProviderCredentialState) -> BoxFuture<'_, Result<(), ProviderStoreError>> {
        unused_future()
    }

    fn read<'a>(
        &'a self,
        _: &'a ProviderAccountId,
    ) -> BoxFuture<'a, Result<Option<ProviderCredentialState>, ProviderStoreError>> {
        unused_future()
    }

    fn clear<'a>(
        &'a self,
        _: &'a ProviderAccountId,
    ) -> BoxFuture<'a, Result<bool, ProviderStoreError>> {
        unused_future()
    }

    fn record_refresh_backoff<'a>(
        &'a self,
        _: &'a ProviderAccountId,
        _: Duration,
    ) -> BoxFuture<'a, Result<u32, ProviderStoreError>> {
        unused_future()
    }

    fn clear_refresh_backoff<'a>(
        &'a self,
        _: &'a ProviderAccountId,
    ) -> BoxFuture<'a, Result<(), ProviderStoreError>> {
        unused_future()
    }
}

impl ProviderCooldownPort for Unused {
    fn put_if_later(&self, _: ProviderCooldown) -> BoxFuture<'_, Result<bool, ProviderStoreError>> {
        unused_future()
    }

    fn read<'a>(
        &'a self,
        _: &'a ProviderAccountId,
    ) -> BoxFuture<'a, Result<Option<ProviderCooldown>, ProviderStoreError>> {
        unused_future()
    }

    fn clear<'a>(
        &'a self,
        _: &'a ProviderAccountId,
        _: CredentialRevision,
    ) -> BoxFuture<'a, Result<bool, ProviderStoreError>> {
        unused_future()
    }

    fn put_scoped_if_later(
        &self,
        _: ProviderScopedCooldown,
    ) -> BoxFuture<'_, Result<bool, ProviderStoreError>> {
        unused_future()
    }

    fn read_scoped<'a>(
        &'a self,
        _: &'a ProviderAccountId,
        _: &'a ProviderCooldownScope,
    ) -> BoxFuture<'a, Result<Option<ProviderScopedCooldown>, ProviderStoreError>> {
        unused_future()
    }

    fn clear_scoped<'a>(
        &'a self,
        _: &'a ProviderAccountId,
        _: &'a ProviderCooldownScope,
        _: CredentialRevision,
    ) -> BoxFuture<'a, Result<bool, ProviderStoreError>> {
        unused_future()
    }

    fn clear_all<'a>(
        &'a self,
        _: &'a ProviderAccountId,
    ) -> BoxFuture<'a, Result<bool, ProviderStoreError>> {
        unused_future()
    }

    fn clear_after_success<'a>(
        &'a self,
        _: &'a ProviderAccountId,
        _: CredentialRevision,
    ) -> BoxFuture<'a, Result<(), ProviderStoreError>> {
        unused_future()
    }
}

impl ProviderRuntimePolicyPort for Unused {
    fn load_refresh_policy(
        &self,
    ) -> BoxFuture<'_, Result<ProviderRefreshPolicy, ProviderStoreError>> {
        unused_future()
    }
}

impl OAuthPendingFlowPort for Unused {
    fn put_if_absent(
        &self,
        _: NewOAuthPendingFlow,
    ) -> BoxFuture<'_, Result<OAuthPendingPutOutcome, ProviderStoreError>> {
        unused_future()
    }

    fn claim_if_owner<'a>(
        &'a self,
        _: &'a ProviderKind,
        _: &'a OAuthPendingBinding,
        _: &'a OAuthPendingBinding,
        _: &'a OAuthPendingBinding,
        _: Duration,
    ) -> BoxFuture<'a, Result<OAuthPendingClaimOutcome, ProviderStoreError>> {
        unused_future()
    }

    fn release_claim<'a>(
        &'a self,
        _: &'a ProviderKind,
        _: &'a OAuthPendingBinding,
        _: &'a OAuthPendingBinding,
        _: &'a OAuthPendingBinding,
    ) -> BoxFuture<'a, Result<OAuthPendingReleaseOutcome, ProviderStoreError>> {
        unused_future()
    }

    fn consume_claim<'a>(
        &'a self,
        _: &'a ProviderKind,
        _: &'a OAuthPendingBinding,
        _: &'a OAuthPendingBinding,
        _: &'a OAuthPendingBinding,
    ) -> BoxFuture<'a, Result<OAuthPendingConsumeOutcome, ProviderStoreError>> {
        unused_future()
    }
}

#[async_trait]
impl OperationalDiagnostics for Unused {
    async fn record_failure(&self, _: OperationalFailure) -> Result<(), StoreError> {
        Ok(())
    }
}
