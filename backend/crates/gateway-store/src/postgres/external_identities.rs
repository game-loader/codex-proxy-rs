//! 外部登录身份绑定的读取与自动开通事务

use async_trait::async_trait;
use chrono::Utc;
use gateway_admin::{
    model::{
        MutationContext,
        audit::MutationAuditOperation,
        client_keys::NewClientKey,
        external_identity::{ExternalIdentity, ExternalIdentityProvider, ExternalKeyProvision},
    },
    ports::{external_identity::ExternalIdentityStore, store::AdminStoreResult},
};
use gateway_core::policy::ClientApiKeyId;
use sqlx::{PgPool, Postgres, Transaction};

use super::{
    append_admin_audit_event_in_transaction, bump_config_revision_in_transaction,
    client_keys::{NewClientApiKey, insert_client_api_key_in_transaction},
};
use crate::{StoreError, admin_revision, admin_store_error, mutation_audit, postgres_unavailable};

const ENTITY: &str = "external identity";

#[derive(Clone)]
pub struct PgExternalIdentityStore {
    pool: PgPool,
}

impl PgExternalIdentityStore {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

async fn bound_key(
    executor: impl sqlx::PgExecutor<'_>,
    provider: ExternalIdentityProvider,
    tenant_key: &str,
    subject: &str,
) -> Result<Option<String>, StoreError> {
    sqlx::query_scalar::<_, String>(
        "select client_api_key_id from client_key_external_identities
         where provider = $1 and tenant_key = $2 and subject = $3",
    )
    .bind(provider.as_str())
    .bind(tenant_key)
    .bind(subject)
    .fetch_optional(executor)
    .await
    .map_err(|source| postgres_unavailable("read external identity binding", source))
}

fn key_id(value: String) -> AdminStoreResult<ClientApiKeyId> {
    ClientApiKeyId::new(value).map_err(|_| {
        admin_store_error(
            ENTITY,
            StoreError::InvalidData {
                entity: ENTITY,
                message: "stored client API key id is invalid".to_owned(),
                source: None,
            },
        )
    })
}

#[async_trait]
impl ExternalIdentityStore for PgExternalIdentityStore {
    async fn find_bound_key(
        &self,
        provider: ExternalIdentityProvider,
        tenant_key: &str,
        subject: &str,
    ) -> AdminStoreResult<Option<ClientApiKeyId>> {
        bound_key(&self.pool, provider, tenant_key, subject)
            .await
            .map_err(|error| admin_store_error(ENTITY, error))?
            .map(key_id)
            .transpose()
    }

    async fn provision_key(
        &self,
        identity: &ExternalIdentity,
        key: NewClientKey,
        max_identities: Option<u64>,
        context: &MutationContext,
    ) -> AdminStoreResult<ExternalKeyProvision> {
        let mut transaction = self.pool.begin().await.map_err(|source| {
            admin_store_error(
                ENTITY,
                postgres_unavailable("begin external key provision", source),
            )
        })?;
        let result =
            provision_in_transaction(&mut transaction, identity, key, max_identities, context)
                .await;
        match result {
            Ok(ExternalKeyProvision::Created {
                client_key_id,
                config_revision,
            }) => {
                transaction.commit().await.map_err(|source| {
                    admin_store_error(
                        ENTITY,
                        postgres_unavailable("commit external key provision", source),
                    )
                })?;
                Ok(ExternalKeyProvision::Created {
                    client_key_id,
                    config_revision,
                })
            }
            // 未写入时回滚递增的配置版本，不产生空发布
            Ok(other) => {
                let _ = transaction.rollback().await;
                Ok(other)
            }
            Err(error) => {
                let _ = transaction.rollback().await;
                Err(error)
            }
        }
    }
}

async fn provision_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    identity: &ExternalIdentity,
    key: NewClientKey,
    max_identities: Option<u64>,
    context: &MutationContext,
) -> AdminStoreResult<ExternalKeyProvision> {
    let map_error = |error| admin_store_error(ENTITY, error);
    // 先递增配置版本取得控制面行锁，复查绑定与人数上限时不会被并发开通穿插
    let revision = bump_config_revision_in_transaction(transaction)
        .await
        .map_err(map_error)?;
    if let Some(existing) = bound_key(
        &mut **transaction,
        identity.provider,
        &identity.tenant_key,
        &identity.subject,
    )
    .await
    .map_err(map_error)?
    {
        return Ok(ExternalKeyProvision::Existing(key_id(existing)?));
    }
    if let Some(limit) = max_identities {
        let count = sqlx::query_scalar::<_, i64>(
            "select count(*) from client_key_external_identities where provider = $1",
        )
        .bind(identity.provider.as_str())
        .fetch_one(&mut **transaction)
        .await
        .map_err(|source| map_error(postgres_unavailable("count external identities", source)))?;
        if u64::try_from(count).unwrap_or(u64::MAX) >= limit {
            return Ok(ExternalKeyProvision::LimitReached);
        }
    }
    let client_key_id = key.id.clone();
    let new_key = NewClientApiKey {
        request_profile_overrides: key.request_profile_overrides,
        id: key.id.as_str().to_owned(),
        name: key.name,
        label: key.label,
        group_ids: key
            .group_ids
            .iter()
            .map(|id| id.as_str().to_owned())
            .collect(),
        key: key.plaintext,
        budget: key.budget,
        max_concurrency: key.limits.max_concurrency,
        requests_per_minute: key.limits.requests_per_minute,
    };
    insert_client_api_key_in_transaction(transaction, &new_key)
        .await
        .map_err(map_error)?;
    sqlx::query(
        "insert into client_key_external_identities
           (provider, tenant_key, subject, client_api_key_id, display_name, created_at)
         values ($1, $2, $3, $4, $5, $6)",
    )
    .bind(identity.provider.as_str())
    .bind(&identity.tenant_key)
    .bind(&identity.subject)
    .bind(client_key_id.as_str())
    .bind(&identity.display_name)
    .bind(Utc::now())
    .execute(&mut **transaction)
    .await
    .map_err(|source| map_error(postgres_unavailable("insert external identity", source)))?;
    let audit = mutation_audit(
        context,
        MutationAuditOperation::ClientApiKeyExternalProvision,
        client_key_id.as_str(),
        vec![
            format!("identity_provider:{}", identity.provider.as_str()),
            "name".to_owned(),
            "group_ids".to_owned(),
            "key".to_owned(),
            "enabled".to_owned(),
            "max_concurrency".to_owned(),
            "requests_per_minute".to_owned(),
            "daily_limit_usd".to_owned(),
            "weekly_limit_usd".to_owned(),
        ],
    );
    append_admin_audit_event_in_transaction(transaction, audit, revision)
        .await
        .map_err(map_error)?;
    Ok(ExternalKeyProvision::Created {
        client_key_id,
        config_revision: admin_revision(revision)?,
    })
}
