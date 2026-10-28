//! 验证外部身份自动开通的事务原子性、并发去重、人数上限与删除 Key 后解除绑定

use gateway_admin::{
    model::{
        MutationActor, MutationContext,
        client_keys::{DeleteClientKey, NewClientKey},
        external_identity::{ExternalIdentity, ExternalIdentityProvider, ExternalKeyProvision},
    },
    ports::{external_identity::ExternalIdentityStore as _, store::ClientKeyStore as _},
};
use gateway_core::policy::{ClientApiKeyId, RateLimits};
use gateway_store::postgres::{PgAdminClientKeyStore, PgExternalIdentityStore};

use super::TestDatabase;

fn identity(subject: &str) -> ExternalIdentity {
    ExternalIdentity {
        provider: ExternalIdentityProvider::Feishu,
        tenant_key: "tenant-a".to_owned(),
        subject: subject.to_owned(),
        display_name: Some("成员".to_owned()),
    }
}

fn key(index: usize) -> NewClientKey {
    NewClientKey {
        request_profile_overrides: Default::default(),
        id: ClientApiKeyId::new(format!("key_feishu_{index}")).unwrap(),
        name: format!("feishu-member-{index}"),
        label: None,
        group_ids: Vec::new(),
        limits: RateLimits::unlimited(),
        budget: Default::default(),
        plaintext: format!("sk_feishu_test_key_{index}_0123456789abcdef"),
    }
}

fn context() -> MutationContext {
    MutationContext {
        actor: MutationActor::System,
        request_id: "feishu-test".to_owned(),
    }
}

#[tokio::test]
async fn concurrent_first_logins_create_one_binding_and_limits_block_new_identities() {
    let Some(database) = TestDatabase::create("external_identity").await else {
        return;
    };
    let store = PgExternalIdentityStore::new(database.pool.clone());
    let member = identity("on_1");
    let context = context();

    let (first, second) = tokio::join!(
        store.provision_key(&member, key(1), Some(1), &context),
        store.provision_key(&member, key(2), Some(1), &context),
    );
    let outcomes = [first.unwrap(), second.unwrap()];
    let created: Vec<_> = outcomes
        .iter()
        .filter_map(|outcome| match outcome {
            ExternalKeyProvision::Created { client_key_id, .. } => Some(client_key_id.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(created.len(), 1);
    assert!(outcomes.contains(&ExternalKeyProvision::Existing(created[0].clone())));
    assert_eq!(
        store
            .find_bound_key(ExternalIdentityProvider::Feishu, "tenant-a", "on_1")
            .await
            .unwrap(),
        Some(created[0].clone())
    );
    let keys: i64 = sqlx::query_scalar("select count(*) from client_api_keys")
        .fetch_one(&database.pool)
        .await
        .unwrap();
    assert_eq!(keys, 1);

    assert_eq!(
        store
            .provision_key(&identity("on_2"), key(3), Some(1), &context)
            .await
            .unwrap(),
        ExternalKeyProvision::LimitReached
    );
    let audits: i64 = sqlx::query_scalar(
        "select count(*) from admin_audit_events where action = 'external_provision'",
    )
    .fetch_one(&database.pool)
    .await
    .unwrap();
    assert_eq!(audits, 1);

    PgAdminClientKeyStore::new(database.pool.clone())
        .delete_client_key(
            DeleteClientKey {
                id: created[0].clone(),
            },
            &context,
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .find_bound_key(ExternalIdentityProvider::Feishu, "tenant-a", "on_1")
            .await
            .unwrap(),
        None
    );
    database.close().await;
}

#[tokio::test]
async fn failed_key_insert_rolls_back_the_binding() {
    let Some(database) = TestDatabase::create("external_identity_rollback").await else {
        return;
    };
    let store = PgExternalIdentityStore::new(database.pool.clone());
    store
        .provision_key(&identity("on_1"), key(1), None, &context())
        .await
        .unwrap();
    // 同名 Key 违反名称唯一约束，绑定不能单独提交
    let mut duplicate = key(2);
    duplicate.name = "feishu-member-1".to_owned();

    assert!(
        store
            .provision_key(&identity("on_2"), duplicate, None, &context())
            .await
            .is_err()
    );
    assert_eq!(
        store
            .find_bound_key(ExternalIdentityProvider::Feishu, "tenant-a", "on_2")
            .await
            .unwrap(),
        None
    );
    database.close().await;
}
