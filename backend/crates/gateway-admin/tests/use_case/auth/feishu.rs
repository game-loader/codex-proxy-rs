//! 验证飞书登录的企业校验、首次开通、绑定复用与停用 Key 的拒绝

use std::{
    collections::{BTreeMap, BTreeSet},
    net::{IpAddr, Ipv4Addr},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration as StdDuration,
};

use async_trait::async_trait;
use gateway_admin::{
    FeishuAppSecret, FeishuConfig, FeishuProvisionConfig,
    model::{
        MutationContext, Revision,
        auth::{AdminAuditEvent, AuthSession, LoginCommand, LoginError, SessionSubject},
        client_keys::NewClientKey,
        external_identity::{ExternalIdentity, ExternalIdentityProvider, ExternalKeyProvision},
        settings::AdminApiKey,
    },
    ports::{
        external_identity::{
            ExternalIdentityExchange, ExternalIdentityStore, IdentityExchangeError,
        },
        store::{AdminStoreResult, AuthStore},
    },
};
use gateway_core::policy::ClientApiKeyId;

const VERIFIER: &str = "0123456789abcdefghijklmnopqrstuvwxyzABCDEFG";

#[derive(Default)]
struct Keys {
    enabled: Mutex<BTreeSet<String>>,
}

struct FixtureExchange {
    identity: Mutex<Result<ExternalIdentity, IdentityExchangeError>>,
}

impl FixtureExchange {
    fn returning(tenant: &str, subject: &str) -> Arc<Self> {
        Arc::new(Self {
            identity: Mutex::new(Ok(ExternalIdentity {
                provider: ExternalIdentityProvider::Feishu,
                tenant_key: tenant.to_owned(),
                subject: subject.to_owned(),
                display_name: Some("张三".to_owned()),
            })),
        })
    }

    fn failing(error: IdentityExchangeError) -> Arc<Self> {
        Arc::new(Self {
            identity: Mutex::new(Err(error)),
        })
    }
}

#[async_trait]
impl ExternalIdentityExchange for FixtureExchange {
    fn authorize_url(&self, state: &str, code_challenge: &str) -> String {
        format!("https://feishu.test/authorize?state={state}&code_challenge={code_challenge}")
    }

    async fn exchange(&self, _: &str, _: &str) -> Result<ExternalIdentity, IdentityExchangeError> {
        self.identity.lock().unwrap().clone()
    }
}

#[derive(Default)]
struct MemoryIdentities {
    keys: Arc<Keys>,
    bindings: Mutex<BTreeMap<(String, String), ClientApiKeyId>>,
    created: Mutex<Vec<NewClientKey>>,
    limits: Mutex<Vec<Option<u64>>>,
    provisions: AtomicUsize,
}

#[async_trait]
impl ExternalIdentityStore for MemoryIdentities {
    async fn find_bound_key(
        &self,
        _: ExternalIdentityProvider,
        tenant_key: &str,
        subject: &str,
    ) -> AdminStoreResult<Option<ClientApiKeyId>> {
        Ok(self
            .bindings
            .lock()
            .unwrap()
            .get(&(tenant_key.to_owned(), subject.to_owned()))
            .cloned())
    }

    async fn provision_key(
        &self,
        identity: &ExternalIdentity,
        key: NewClientKey,
        max_identities: Option<u64>,
        _: &MutationContext,
    ) -> AdminStoreResult<ExternalKeyProvision> {
        self.provisions.fetch_add(1, Ordering::Relaxed);
        self.limits.lock().unwrap().push(max_identities);
        let mut bindings = self.bindings.lock().unwrap();
        if max_identities.is_some_and(|limit| bindings.len() as u64 >= limit) {
            return Ok(ExternalKeyProvision::LimitReached);
        }
        let id = key.id.clone();
        bindings.insert(
            (identity.tenant_key.clone(), identity.subject.clone()),
            id.clone(),
        );
        self.keys
            .enabled
            .lock()
            .unwrap()
            .insert(id.as_str().to_owned());
        self.created.lock().unwrap().push(key);
        Ok(ExternalKeyProvision::Created {
            client_key_id: id,
            config_revision: Revision::new(2).unwrap(),
        })
    }
}

struct SessionStore {
    keys: Arc<Keys>,
    sessions: Mutex<BTreeMap<String, AuthSession>>,
}

#[async_trait]
impl AuthStore for SessionStore {
    async fn load_password_hash(&self, _: &str) -> AdminStoreResult<Option<String>> {
        Ok(None)
    }
    async fn change_password(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: AdminAuditEvent,
    ) -> AdminStoreResult<bool> {
        Ok(false)
    }
    async fn create_password_hash_if_absent(&self, _: &str, _: &str) -> AdminStoreResult<bool> {
        Ok(true)
    }
    async fn load_admin_api_key(&self) -> AdminStoreResult<Option<AdminApiKey>> {
        Ok(None)
    }
    async fn load_session(&self, id: &str) -> AdminStoreResult<Option<AuthSession>> {
        Ok(self.sessions.lock().unwrap().get(id).cloned())
    }
    async fn store_session(&self, id: &str, session: &AuthSession) -> AdminStoreResult<()> {
        self.sessions
            .lock()
            .unwrap()
            .insert(id.to_owned(), session.clone());
        Ok(())
    }
    async fn compare_exchange_session(
        &self,
        _: &str,
        _: &AuthSession,
        _: Option<&AuthSession>,
    ) -> AdminStoreResult<Option<AuthSession>> {
        Ok(None)
    }
    async fn delete_session(&self, id: &str) -> AdminStoreResult<Option<AuthSession>> {
        Ok(self.sessions.lock().unwrap().remove(id))
    }
    async fn client_key_enabled(&self, id: &ClientApiKeyId) -> AdminStoreResult<bool> {
        Ok(self.keys.enabled.lock().unwrap().contains(id.as_str()))
    }
    async fn consume_login_attempt(
        &self,
        _: IpAddr,
        _: u32,
        _: u32,
        _: StdDuration,
    ) -> AdminStoreResult<Option<StdDuration>> {
        Ok(None)
    }
    async fn append_audit_event(&self, _: AdminAuditEvent) -> AdminStoreResult<()> {
        Ok(())
    }
}

fn config(provision: FeishuProvisionConfig) -> FeishuConfig {
    FeishuConfig {
        app_id: "cli_test".to_owned(),
        app_secret: FeishuAppSecret::new("secret"),
        redirect_uri: "https://proxy.example.com/api/auth/feishu/callback".to_owned(),
        allowed_tenant_keys: vec!["tenant-a".to_owned()],
        provision,
    }
}

async fn services(
    exchange: Arc<FixtureExchange>,
    identities: Arc<MemoryIdentities>,
    provision: FeishuProvisionConfig,
) -> gateway_admin::AdminServices {
    let auth = Arc::new(SessionStore {
        keys: identities.keys.clone(),
        sessions: Mutex::new(BTreeMap::new()),
    });
    super::super::AdminHarness::new()
        .auth(auth)
        .feishu(config(provision), exchange, identities)
        .build()
        .await
}

fn login() -> LoginCommand {
    LoginCommand::Feishu {
        code: "code".to_owned(),
        code_verifier: VERIFIER.to_owned(),
    }
}

async fn sign_in(services: &gateway_admin::AdminServices) -> Result<SessionSubject, LoginError> {
    services
        .auth()
        .login(login(), Ipv4Addr::LOCALHOST.into(), None)
        .await
        .map(|grant| grant.session.subject)
}

#[tokio::test]
async fn first_login_provisions_one_key_from_template_and_later_logins_reuse_it() {
    let identities = Arc::new(MemoryIdentities::default());
    let provision = FeishuProvisionConfig {
        max_concurrency: 2,
        requests_per_minute: 30,
        daily_limit_usd: "5".to_owned(),
        ..FeishuProvisionConfig::default()
    };
    let services = services(
        FixtureExchange::returning("tenant-a", "on_1"),
        identities.clone(),
        provision,
    )
    .await;

    let first = sign_in(&services).await.expect("first login");
    let second = sign_in(&services).await.expect("second login");

    assert_eq!(first, second);
    assert_eq!(identities.provisions.load(Ordering::Relaxed), 1);
    let created = identities.created.lock().unwrap();
    let key = &created[0];
    assert!(matches!(first, SessionSubject::Key { ref client_key_id } if *client_key_id == key.id));
    assert_eq!(key.limits.max_concurrency, 2);
    assert_eq!(key.limits.requests_per_minute, 30);
    assert_eq!(key.budget.daily_usd, "5".parse().unwrap());
    assert!(key.name.starts_with("feishu-张三-"));
    assert!(key.plaintext.starts_with("sk_"));
}

#[tokio::test]
async fn other_tenants_and_closed_provisioning_are_denied_without_writes() {
    let identities = Arc::new(MemoryIdentities::default());
    let foreign = services(
        FixtureExchange::returning("tenant-b", "on_1"),
        identities.clone(),
        FeishuProvisionConfig::default(),
    )
    .await;
    assert_eq!(sign_in(&foreign).await, Err(LoginError::AccessDenied));

    let closed = services(
        FixtureExchange::returning("tenant-a", "on_1"),
        identities.clone(),
        FeishuProvisionConfig {
            enabled: false,
            ..FeishuProvisionConfig::default()
        },
    )
    .await;
    assert_eq!(sign_in(&closed).await, Err(LoginError::AccessDenied));
    assert_eq!(identities.provisions.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn disabled_bound_key_and_user_limit_reject_login() {
    let identities = Arc::new(MemoryIdentities::default());
    let services = services(
        FixtureExchange::returning("tenant-a", "on_1"),
        identities.clone(),
        FeishuProvisionConfig {
            max_users: Some(1),
            ..FeishuProvisionConfig::default()
        },
    )
    .await;
    sign_in(&services).await.expect("provision");
    identities.keys.enabled.lock().unwrap().clear();

    assert_eq!(sign_in(&services).await, Err(LoginError::AccessDenied));
    assert_eq!(identities.provisions.load(Ordering::Relaxed), 1);
    assert_eq!(*identities.limits.lock().unwrap(), vec![Some(1)]);

    let other = services_with_shared(&identities, "on_2").await;
    assert_eq!(sign_in(&other).await, Err(LoginError::AccessDenied));
}

async fn services_with_shared(
    identities: &Arc<MemoryIdentities>,
    subject: &str,
) -> gateway_admin::AdminServices {
    services(
        FixtureExchange::returning("tenant-a", subject),
        identities.clone(),
        FeishuProvisionConfig {
            max_users: Some(1),
            ..FeishuProvisionConfig::default()
        },
    )
    .await
}

#[tokio::test]
async fn exchange_failures_and_malformed_verifiers_map_to_login_errors() {
    for (error, expected) in [
        (
            IdentityExchangeError::Rejected,
            LoginError::InvalidCredentials,
        ),
        (IdentityExchangeError::Unavailable, LoginError::Unavailable),
    ] {
        let services = services(
            FixtureExchange::failing(error),
            Arc::new(MemoryIdentities::default()),
            FeishuProvisionConfig::default(),
        )
        .await;
        assert_eq!(sign_in(&services).await, Err(expected));
    }

    let services = services(
        FixtureExchange::returning("tenant-a", "on_1"),
        Arc::new(MemoryIdentities::default()),
        FeishuProvisionConfig::default(),
    )
    .await;
    let short = services
        .auth()
        .login(
            LoginCommand::Feishu {
                code: "code".to_owned(),
                code_verifier: "short".to_owned(),
            },
            Ipv4Addr::LOCALHOST.into(),
            None,
        )
        .await;
    assert_eq!(short.map(|_| ()), Err(LoginError::InvalidCredentials));
}

#[tokio::test]
async fn authorization_uses_fresh_state_and_s256_challenge_of_the_verifier() {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use sha2::{Digest as _, Sha256};

    let services = services(
        FixtureExchange::returning("tenant-a", "on_1"),
        Arc::new(MemoryIdentities::default()),
        FeishuProvisionConfig::default(),
    )
    .await;
    let first = services.auth().feishu_authorization().expect("configured");
    let second = services.auth().feishu_authorization().expect("configured");

    assert_ne!(first.state, second.state);
    assert_eq!(first.code_verifier.len(), 43);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(first.code_verifier.as_bytes()));
    assert!(first.url.contains(&format!("state={}", first.state)));
    assert!(first.url.contains(&format!("code_challenge={challenge}")));
    assert!(first.secure_callback);
    assert!(!format!("{first:?}").contains(&first.code_verifier));
}

#[tokio::test]
async fn feishu_login_is_unavailable_without_configuration() {
    let services = super::super::AdminHarness::new().build().await;

    assert!(services.auth().feishu_authorization().is_none());
    assert_eq!(
        services
            .auth()
            .login(login(), Ipv4Addr::LOCALHOST.into(), None)
            .await
            .map(|_| ()),
        Err(LoginError::InvalidCredentials)
    );
}
