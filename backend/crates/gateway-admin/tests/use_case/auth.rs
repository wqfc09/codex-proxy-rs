use std::{collections::BTreeMap, sync::Mutex};

use async_trait::async_trait;
use chrono::{TimeDelta, Utc};

use gateway_admin::{
    model::{
        auth::{AdminAuditEvent, AuthSession, LoginCommand},
        settings::AdminApiKey,
        users::{UserCredentialRecord, UserRecord, UserRole},
    },
    ports::store::{AdminStoreResult, AuthStore},
};
use gateway_core::policy::UserRateLimits;

#[derive(Default)]
struct MemoryAuthStore {
    turnstile_enabled: std::sync::atomic::AtomicBool,
    retry_after: Mutex<Option<std::time::Duration>>,
    unavailable: std::sync::atomic::AtomicBool,
    reject_delete: Mutex<Option<String>>,
    password_hash: Mutex<Option<String>>,
    sessions: Mutex<BTreeMap<String, AuthSession>>,
    audits: Mutex<Vec<AdminAuditEvent>>,
}

#[tokio::test]
async fn convenience_login_cannot_bypass_enabled_turnstile_without_proof() {
    let store = std::sync::Arc::new(MemoryAuthStore::default());
    let services = super::AdminHarness::new().auth(store.clone()).build().await;
    store
        .turnstile_enabled
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let result = services
        .auth()
        .login(
            LoginCommand {
                username: None,
                password: "strong-test-password".to_owned(),
            },
            std::net::Ipv4Addr::LOCALHOST.into(),
            None,
        )
        .await;
    assert_eq!(
        result,
        Err(gateway_admin::model::auth::LoginError::InvalidCredentials)
    );
    assert!(store.sessions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn password_change_obeys_rate_limit_and_rejects_unbound_legacy_sessions() {
    use gateway_admin::model::{
        AdminErrorKind,
        auth::{ChangePassword, SessionSubject},
    };
    let store = std::sync::Arc::new(MemoryAuthStore::default());
    let services = super::AdminHarness::new().auth(store.clone()).build().await;
    let auth = services.auth();
    let login = auth
        .login(
            LoginCommand {
                username: None,
                password: "strong-test-password".into(),
            },
            std::net::Ipv4Addr::LOCALHOST.into(),
            None,
        )
        .await
        .unwrap();
    *store.retry_after.lock().unwrap() = Some(std::time::Duration::from_secs(30));
    let error = auth
        .change_password(
            Some(&login.session_id),
            ChangePassword {
                current_password: "strong-test-password".into(),
                new_password: "new-strong-password".into(),
            },
            std::net::Ipv4Addr::LOCALHOST.into(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind(), AdminErrorKind::RateLimited);
    assert!(
        auth.session(Some(&login.session_id))
            .await
            .unwrap()
            .is_some()
    );
    store.sessions.lock().unwrap().insert(
        "legacy".into(),
        AuthSession {
            subject: SessionSubject::Admin {
                admin_user_id: "admin".into(),
                credential_fingerprint: String::new(),
            },
            expires_at: Utc::now() + TimeDelta::hours(1),
        },
    );
    assert!(auth.session(Some("legacy")).await.unwrap().is_none());
}

#[async_trait]
impl AuthStore for MemoryAuthStore {
    async fn load_turnstile_settings(
        &self,
    ) -> AdminStoreResult<gateway_admin::model::users::TurnstileSettings> {
        Ok(gateway_admin::model::users::TurnstileSettings::new(
            self.turnstile_enabled
                .load(std::sync::atomic::Ordering::SeqCst),
            Some("test-site".to_owned()),
            Some("test-secret".to_owned()),
        ))
    }
    async fn load_user_by_username(
        &self,
        username: &str,
    ) -> AdminStoreResult<Option<UserCredentialRecord>> {
        if username != "admin" {
            return Ok(None);
        }
        Ok(self
            .password_hash
            .lock()
            .expect("password hash")
            .clone()
            .map(|password_hash| UserCredentialRecord {
                user: UserRecord {
                    id: "admin".to_owned(),
                    username: "admin".to_owned(),
                    role: UserRole::Admin,
                    enabled: true,
                    limits: UserRateLimits::default(),
                    session_version: 1,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                },
                password_hash,
            }))
    }

    async fn load_user_by_id(
        &self,
        user_id: &str,
    ) -> AdminStoreResult<Option<UserCredentialRecord>> {
        self.load_user_by_username(user_id).await
    }

    async fn load_password_hash(&self, _: &str) -> AdminStoreResult<Option<String>> {
        Ok(self.password_hash.lock().expect("password hash").clone())
    }

    async fn change_password(
        &self,
        _: &str,
        expected_hash: &str,
        password_hash: &str,
        audit: gateway_admin::model::auth::AdminAuditEvent,
    ) -> AdminStoreResult<bool> {
        let mut stored = self.password_hash.lock().unwrap();
        let Some(credentials) = stored
            .as_mut()
            .filter(|value| value.as_str() == expected_hash)
        else {
            return Ok(false);
        };
        *credentials = password_hash.to_owned();
        self.audits.lock().unwrap().push(audit);
        Ok(true)
    }

    async fn create_password_hash_if_absent(
        &self,
        _: &str,
        password_hash: &str,
    ) -> AdminStoreResult<bool> {
        let mut stored = self.password_hash.lock().expect("password hash");
        if stored.is_some() {
            return Ok(false);
        }
        *stored = Some(password_hash.to_owned());
        Ok(true)
    }

    async fn load_admin_api_key(&self) -> AdminStoreResult<Option<AdminApiKey>> {
        Ok(None)
    }

    async fn load_session(&self, session_id: &str) -> AdminStoreResult<Option<AuthSession>> {
        if self.unavailable.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(super::unavailable("session"));
        }
        Ok(self
            .sessions
            .lock()
            .expect("sessions")
            .get(session_id)
            .cloned())
    }

    async fn store_session(&self, session_id: &str, session: &AuthSession) -> AdminStoreResult<()> {
        self.sessions
            .lock()
            .expect("sessions")
            .insert(session_id.to_owned(), session.clone());
        Ok(())
    }

    async fn delete_session(&self, session_id: &str) -> AdminStoreResult<Option<AuthSession>> {
        if self.reject_delete.lock().unwrap().as_deref() == Some(session_id) {
            return Err(super::unavailable("delete session"));
        }
        Ok(self.sessions.lock().expect("sessions").remove(session_id))
    }

    async fn client_key_enabled(
        &self,
        _: &gateway_core::policy::ClientApiKeyId,
    ) -> AdminStoreResult<bool> {
        Ok(false)
    }

    async fn consume_login_attempt(
        &self,
        _: std::net::IpAddr,
        _: u32,
        _: u32,
        _: std::time::Duration,
    ) -> AdminStoreResult<Option<std::time::Duration>> {
        Ok(*self.retry_after.lock().unwrap())
    }

    async fn append_audit_event(&self, event: AdminAuditEvent) -> AdminStoreResult<()> {
        self.audits.lock().expect("audits").push(event);
        Ok(())
    }
}

#[tokio::test]
async fn successful_login_should_create_expiring_session_and_audit() {
    let store = std::sync::Arc::new(MemoryAuthStore::default());
    let services = super::AdminHarness::new().auth(store.clone()).build().await;

    let result = services
        .auth()
        .login(
            LoginCommand {
                username: Some("admin".to_owned()),
                password: "strong-test-password".to_owned(),
            },
            std::net::Ipv4Addr::LOCALHOST.into(),
            None,
        )
        .await
        .expect("login");

    assert!(
        services
            .auth()
            .session(Some(&result.session_id))
            .await
            .expect("validate")
            .is_some()
    );
    assert_eq!(store.audits.lock().expect("audits").len(), 1);
}

#[tokio::test]
async fn repeated_default_initialization_should_not_replace_password() {
    let store = std::sync::Arc::new(MemoryAuthStore::default());
    super::AdminHarness::new()
        .auth(store.clone())
        .default_password("first-strong-password")
        .build()
        .await;
    let services = super::AdminHarness::new()
        .auth(store)
        .default_password("second-strong-password")
        .build()
        .await;

    assert!(
        services
            .auth()
            .login(
                LoginCommand {
                    username: Some("admin".to_owned()),
                    password: "first-strong-password".to_owned(),
                },
                std::net::Ipv4Addr::LOCALHOST.into(),
                None
            )
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn login_with_huge_session_ttl_should_clamp_expiry_instead_of_panicking() {
    let store = std::sync::Arc::new(MemoryAuthStore::default());
    let services = super::AdminHarness::new()
        .auth(store.clone())
        .session_ttl_minutes(i64::MAX as u64)
        .build()
        .await;

    let before = Utc::now();
    let result = services
        .auth()
        .login(
            LoginCommand {
                username: Some("admin".to_owned()),
                password: "strong-test-password".to_owned(),
            },
            std::net::Ipv4Addr::LOCALHOST.into(),
            None,
        )
        .await
        .expect("login with huge session TTL");

    assert!(result.session.expires_at > before);
    assert!(result.session.expires_at <= before + TimeDelta::days(366) + TimeDelta::minutes(1));
    assert!(
        services
            .auth()
            .session(Some(&result.session_id))
            .await
            .expect("validate")
            .is_some()
    );
}

#[tokio::test]
async fn expired_sessions_are_removed_and_store_outages_are_not_treated_as_logout() {
    use gateway_admin::model::{AdminErrorKind, auth::SessionSubject};
    let store = std::sync::Arc::new(MemoryAuthStore::default());
    let services = super::AdminHarness::new().auth(store.clone()).build().await;
    store.sessions.lock().unwrap().insert(
        "expired".to_owned(),
        AuthSession {
            subject: SessionSubject::Admin {
                credential_fingerprint: String::new(),
                admin_user_id: "admin".to_owned(),
            },
            expires_at: Utc::now() - TimeDelta::seconds(1),
        },
    );
    assert!(
        services
            .auth()
            .session(Some("expired"))
            .await
            .unwrap()
            .is_none()
    );
    assert!(store.sessions.lock().unwrap().is_empty());
    store
        .unavailable
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        services
            .auth()
            .session(Some("existing"))
            .await
            .unwrap_err()
            .kind(),
        AdminErrorKind::Unavailable
    );
}

#[tokio::test]
async fn inactive_legacy_key_session_is_revoked_without_restoring_key_login() {
    use gateway_admin::model::auth::SessionSubject;
    let store = std::sync::Arc::new(MemoryAuthStore::default());
    store.sessions.lock().unwrap().insert(
        "legacy-key".to_owned(),
        AuthSession {
            subject: SessionSubject::Key {
                client_key_id: gateway_core::policy::ClientApiKeyId::new("key-legacy").unwrap(),
            },
            expires_at: Utc::now() + TimeDelta::hours(1),
        },
    );
    let services = super::AdminHarness::new().auth(store.clone()).build().await;

    assert!(
        services
            .auth()
            .session(Some("legacy-key"))
            .await
            .unwrap()
            .is_none()
    );
    assert!(!store.sessions.lock().unwrap().contains_key("legacy-key"));
}

#[tokio::test]
async fn login_limit_rejects_before_account_credentials_are_checked() {
    use gateway_admin::model::auth::LoginError;
    let store = std::sync::Arc::new(MemoryAuthStore::default());
    *store.retry_after.lock().unwrap() = Some(std::time::Duration::from_secs(37));
    let services = super::AdminHarness::new().auth(store).build().await;
    let error = services
        .auth()
        .login(
            LoginCommand {
                username: None,
                password: "wrong-password".to_owned(),
            },
            std::net::Ipv4Addr::LOCALHOST.into(),
            None,
        )
        .await
        .unwrap_err();
    assert_eq!(
        error,
        LoginError::TooManyAttempts {
            retry_after_seconds: 37
        }
    );
}
#[tokio::test]
async fn failed_rotation_discards_the_new_session_and_leaves_the_old_session_retryable() {
    use gateway_admin::model::auth::LoginError;
    let store = std::sync::Arc::new(MemoryAuthStore::default());
    let services = super::AdminHarness::new().auth(store.clone()).build().await;
    let command = || LoginCommand {
        username: None,
        password: "strong-test-password".to_owned(),
    };
    let original = services
        .auth()
        .login(command(), std::net::Ipv4Addr::LOCALHOST.into(), None)
        .await
        .unwrap();
    assert!(!format!("{original:?}").contains(&original.session_id));
    *store.reject_delete.lock().unwrap() = Some(original.session_id.clone());
    assert_eq!(
        services
            .auth()
            .login(
                command(),
                std::net::Ipv4Addr::LOCALHOST.into(),
                Some(&original.session_id)
            )
            .await
            .unwrap_err(),
        LoginError::Unavailable
    );
    assert_eq!(store.sessions.lock().unwrap().len(), 1);
    assert!(
        services
            .auth()
            .session(Some(&original.session_id))
            .await
            .unwrap()
            .is_some()
    );
    assert!(services.auth().logout(&original.session_id).await.is_err());
    *store.reject_delete.lock().unwrap() = None;
    services.auth().logout(&original.session_id).await.unwrap();
    services.auth().logout(&original.session_id).await.unwrap();
    assert!(store.sessions.lock().unwrap().is_empty());
}
