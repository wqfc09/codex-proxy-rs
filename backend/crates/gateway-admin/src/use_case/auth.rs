//! 控制面登录、会话恢复与身份校验的唯一 owner。

use std::{net::IpAddr, sync::Arc, time::Duration as StdDuration};

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{Duration, Utc};
use rand_core::{OsRng, RngCore as _};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq as _;
use uuid::Uuid;

use crate::{
    model::{
        AdminError, AdminErrorKind,
        auth::{
            AdminAuditEvent, AuditActorKind, AuthSession, ChangePassword, LoginCommand, LoginError,
            LoginResult, SessionSubject,
        },
        users::{LoginProtectionProof, PublicAuthSettings, UserRecord, UserRole},
    },
    ports::{auth::TurnstileVerifier, store::AuthStore},
};

use super::map_store_error;

/// 所有控制面接口消费同一个会话服务，权限由服务端身份决定。
#[async_trait]
pub trait AuthService: Send + Sync {
    async fn change_password(
        &self,
        session_id: Option<&str>,
        command: ChangePassword,
        source_ip: IpAddr,
    ) -> Result<(), AdminError>;
    async fn ensure_default_admin(&self, password: &str) -> Result<bool, AdminError>;
    async fn session(&self, session_id: Option<&str>) -> Result<Option<AuthSession>, AdminError>;
    async fn resolve_admin_user_id(
        &self,
        session_id: Option<&str>,
    ) -> Result<Option<String>, AdminError>;
    async fn resolve_user(
        &self,
        session_id: Option<&str>,
    ) -> Result<Option<UserRecord>, AdminError>;
    async fn verify_admin_api_key(&self, key: &str) -> Result<bool, AdminError>;
    async fn login(
        &self,
        command: LoginCommand,
        source_ip: IpAddr,
        previous_session_id: Option<&str>,
    ) -> Result<LoginResult, LoginError>;
    async fn login_with_protection(
        &self,
        command: LoginCommand,
        protection: LoginProtectionProof,
        previous_session_id: Option<&str>,
    ) -> Result<LoginResult, LoginError>;
    async fn logout(&self, session_id: &str) -> Result<(), AdminError>;
    async fn public_auth_settings(&self) -> Result<PublicAuthSettings, AdminError>;
}

const MAX_SESSION_TTL_MINUTES: i64 = 366 * 24 * 60;
const LOGIN_WINDOW: StdDuration = StdDuration::from_secs(60);
const LOGIN_ATTEMPTS_PER_SOURCE: u32 = 10;
const LOGIN_ATTEMPTS_GLOBAL: u32 = 200;

pub(crate) struct DefaultAuthService {
    default_admin_user_id: String,
    admin_session_ttl: Duration,
    user_session_ttl: Duration,
    store: Arc<dyn AuthStore>,
    turnstile: Arc<dyn TurnstileVerifier>,
}

impl DefaultAuthService {
    #[must_use]
    pub(crate) fn new(
        default_admin_user_id: impl Into<String>,
        admin_session_ttl_minutes: u64,
        user_session_ttl_minutes: u64,
        store: Arc<dyn AuthStore>,
        turnstile: Arc<dyn TurnstileVerifier>,
    ) -> Self {
        Self {
            default_admin_user_id: default_admin_user_id.into(),
            admin_session_ttl: session_ttl(admin_session_ttl_minutes),
            user_session_ttl: session_ttl(user_session_ttl_minutes),
            store,
            turnstile,
        }
    }

    fn auth_audit(&self, action: &str, admin_user_id: &str) -> AdminAuditEvent {
        AdminAuditEvent {
            id: format!("audit_{}", Uuid::now_v7().simple()),
            actor_kind: AuditActorKind::AdminSession,
            actor_admin_user_id: Some(admin_user_id.to_owned()),
            actor_ref: crate::model::auth::admin_session_actor_ref(admin_user_id),
            request_id: None,
            action: action.to_owned(),
            entity_kind: "admin_session".to_owned(),
            entity_ref: admin_user_id.to_owned(),
            config_revision: None,
            changed_fields: Vec::new(),
            occurred_at: Utc::now(),
        }
    }

    async fn authenticate(&self, command: LoginCommand) -> Result<SessionSubject, LoginError> {
        let LoginCommand { username, password } = command;
        let username = match username.as_deref() {
            Some(value) => {
                let value = value.trim();
                if value.is_empty() {
                    return Err(LoginError::InvalidCredentials);
                }
                value
            }
            None => self.default_admin_user_id.as_str(),
        };
        let credential = self
            .store
            .load_user_by_username(username)
            .await
            .map_err(|_| LoginError::Unavailable)?
            .ok_or(LoginError::InvalidCredentials)?;
        if !credential.user.enabled
            || password.len() > 4096
            || !verify_admin_password(&password, &credential.password_hash)
                .map_err(|_| LoginError::Unavailable)?
        {
            return Err(LoginError::InvalidCredentials);
        }
        let fingerprint =
            session_fingerprint(&credential.password_hash, credential.user.session_version);
        Ok(match credential.user.role {
            crate::model::users::UserRole::Admin => SessionSubject::Admin {
                admin_user_id: credential.user.id,
                credential_fingerprint: fingerprint,
            },
            crate::model::users::UserRole::User => SessionSubject::User {
                user_id: credential.user.id,
                credential_fingerprint: fingerprint,
            },
        })
    }

    async fn ensure_login_protection(
        &self,
        protection: &LoginProtectionProof,
    ) -> Result<(), LoginError> {
        let settings = self
            .store
            .load_turnstile_settings()
            .await
            .map_err(|_| LoginError::Unavailable)?;
        if !settings.enabled {
            return Ok(());
        }
        if settings.site_key.as_deref().is_none_or(str::is_empty) {
            return Err(LoginError::Unavailable);
        }
        let secret = settings
            .secret_for_verification()
            .filter(|value| !value.is_empty())
            .ok_or(LoginError::Unavailable)?;
        let token = protection
            .token
            .as_deref()
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .ok_or(LoginError::InvalidCredentials)?;
        match self
            .turnstile
            .verify(secret, token, protection.remote_ip)
            .await
        {
            Ok(true) => Ok(()),
            Ok(false) => Err(LoginError::InvalidCredentials),
            Err(_) => Err(LoginError::Unavailable),
        }
    }

    async fn login_inner(
        &self,
        command: LoginCommand,
        source_ip: IpAddr,
        previous_session_id: Option<&str>,
        protection: LoginProtectionProof,
    ) -> Result<LoginResult, LoginError> {
        if let Some(retry_after) = self
            .store
            .consume_login_attempt(
                source_ip,
                LOGIN_ATTEMPTS_PER_SOURCE,
                LOGIN_ATTEMPTS_GLOBAL,
                LOGIN_WINDOW,
            )
            .await
            .map_err(|_| LoginError::Unavailable)?
        {
            return Err(LoginError::TooManyAttempts {
                retry_after_seconds: retry_after.as_secs().max(1),
            });
        }
        self.ensure_login_protection(&protection).await?;
        let subject = self.authenticate(command).await?;
        let ttl_override = self
            .store
            .load_session_ttl_settings()
            .await
            .map_err(|_| LoginError::Unavailable)?;
        let ttl = match (&subject, ttl_override) {
            (SessionSubject::Admin { .. }, Some(settings)) => session_ttl(settings.admin_minutes),
            (SessionSubject::User { .. }, Some(settings)) => session_ttl(settings.user_minutes),
            (SessionSubject::Admin { .. }, None) => self.admin_session_ttl,
            (SessionSubject::User { .. }, None) => self.user_session_ttl,
            (SessionSubject::Key { .. }, _) => return Err(LoginError::Unavailable),
        };
        let session = AuthSession {
            subject,
            expires_at: Utc::now() + ttl,
        };
        let session_id = random_session_token();
        self.store
            .store_session(&session_id, &session)
            .await
            .map_err(|_| LoginError::Unavailable)?;
        let login_audit = match &session.subject {
            SessionSubject::Admin { admin_user_id, .. } => {
                Some(self.auth_audit("admin.login", admin_user_id))
            }
            SessionSubject::User { user_id, .. } => {
                let mut audit = self.auth_audit("user.login", user_id);
                audit.actor_kind = crate::model::auth::AuditActorKind::UserSession;
                audit.actor_admin_user_id = None;
                audit.actor_ref = format!("user:{user_id}");
                audit.entity_kind = "user".to_owned();
                Some(audit)
            }
            SessionSubject::Key { .. } => None,
        };
        if let Some(audit) = login_audit
            && self.store.append_audit_event(audit).await.is_err()
        {
            let _ = self.store.delete_session(&session_id).await;
            return Err(LoginError::Unavailable);
        }
        if let Some(previous) = previous_session_id.filter(|value| !value.is_empty())
            && self.logout(previous).await.is_err()
        {
            let _ = self.store.delete_session(&session_id).await;
            return Err(LoginError::Unavailable);
        }
        Ok(LoginResult {
            session_id,
            session,
        })
    }
}

#[async_trait]
impl AuthService for DefaultAuthService {
    async fn change_password(
        &self,
        session_id: Option<&str>,
        command: ChangePassword,
        source_ip: IpAddr,
    ) -> Result<(), AdminError> {
        let session = self
            .session(session_id)
            .await?
            .ok_or_else(|| AdminError::new(AdminErrorKind::Unauthorized, "请先登录"))?;
        let SessionSubject::Admin {
            admin_user_id,
            credential_fingerprint,
        } = session.subject
        else {
            return Err(AdminError::new(
                AdminErrorKind::Forbidden,
                "仅管理员可以修改密码",
            ));
        };
        if self
            .store
            .consume_login_attempt(
                source_ip,
                LOGIN_ATTEMPTS_PER_SOURCE,
                LOGIN_ATTEMPTS_GLOBAL,
                LOGIN_WINDOW,
            )
            .await
            .map_err(|error| map_store_error(error, "password change limit"))?
            .is_some()
        {
            return Err(AdminError::new(
                AdminErrorKind::RateLimited,
                "尝试过于频繁，请稍后再试",
            ));
        }
        super::password::validate_password(&command.new_password)?;
        let credential = self
            .store
            .load_user_by_id(&admin_user_id)
            .await
            .map_err(|error| map_store_error(error, "administrator"))?
            .filter(|credential| {
                credential.user.enabled
                    && credential.user.role == UserRole::Admin
                    && session_fingerprint_matches(
                        &credential_fingerprint,
                        &credential.password_hash,
                        credential.user.session_version,
                    )
            })
            .ok_or_else(|| AdminError::conflict("密码已变更，请重新登录"))?;
        let password_hash = credential.password_hash;
        if command.current_password.len() > 4096
            || !verify_admin_password(&command.current_password, &password_hash)?
        {
            return Err(AdminError::invalid("当前密码不正确"));
        }
        if command.new_password == command.current_password {
            return Err(AdminError::invalid("新密码不能与当前密码相同"));
        }
        let hash = hash_admin_password(&command.new_password)?;
        let mut audit = self.auth_audit("admin.password_changed", &admin_user_id);
        audit.entity_kind = "admin_user".to_owned();
        audit.changed_fields = vec!["password".to_owned()];
        if !self
            .store
            .change_password(&admin_user_id, &password_hash, &hash, audit)
            .await
            .map_err(|error| map_store_error(error, "administrator password"))?
        {
            return Err(AdminError::conflict("密码已变更，请重新登录"));
        }
        // 密码事务提交后旧指纹不再匹配，会话撤销不依赖 Redis 删除成功。
        Ok(())
    }

    async fn ensure_default_admin(&self, password: &str) -> Result<bool, AdminError> {
        let hash = hash_admin_password(password)?;
        self.store
            .create_password_hash_if_absent(&self.default_admin_user_id, &hash)
            .await
            .map_err(|error| map_store_error(error, "administrator"))
    }

    async fn session(&self, session_id: Option<&str>) -> Result<Option<AuthSession>, AdminError> {
        let Some(session_id) = session_id.filter(|value| !value.is_empty()) else {
            return Ok(None);
        };
        let Some(session) = self
            .store
            .load_session(session_id)
            .await
            .map_err(|error| map_store_error(error, "authentication session"))?
        else {
            return Ok(None);
        };
        if session.expires_at <= Utc::now() {
            let _ = self.store.delete_session(session_id).await;
            return Ok(None);
        }
        if let Some((user_id, role, fingerprint)) = session_user_binding(&session.subject) {
            let credential = self
                .store
                .load_user_by_id(user_id)
                .await
                .map_err(|error| map_store_error(error, "user session"))?;
            let valid = credential.as_ref().is_some_and(|credential| {
                credential.user.enabled
                    && credential.user.role == role
                    && session_fingerprint_matches(
                        fingerprint,
                        &credential.password_hash,
                        credential.user.session_version,
                    )
            });
            if !valid {
                let _ = self.store.delete_session(session_id).await;
                return Ok(None);
            }
        }
        if let SessionSubject::Key { client_key_id } = &session.subject
            && !self
                .store
                .client_key_enabled(client_key_id)
                .await
                .map_err(|error| map_store_error(error, "session key"))?
        {
            let _ = self.store.delete_session(session_id).await;
            return Ok(None);
        }
        Ok(Some(session))
    }

    async fn resolve_admin_user_id(
        &self,
        session_id: Option<&str>,
    ) -> Result<Option<String>, AdminError> {
        match self
            .session(session_id)
            .await?
            .map(|session| session.subject)
        {
            Some(SessionSubject::Admin { admin_user_id, .. }) => Ok(Some(admin_user_id)),
            Some(SessionSubject::User { .. }) | Some(SessionSubject::Key { .. }) => Err(
                AdminError::new(AdminErrorKind::Forbidden, "当前身份无权访问管理接口"),
            ),
            None => Ok(None),
        }
    }

    async fn resolve_user(
        &self,
        session_id: Option<&str>,
    ) -> Result<Option<UserRecord>, AdminError> {
        let Some(session) = self.session(session_id).await? else {
            return Ok(None);
        };
        let user_id = match session.subject {
            SessionSubject::Admin { admin_user_id, .. } => admin_user_id,
            SessionSubject::User { user_id, .. } => user_id,
            SessionSubject::Key { .. } => return Ok(None),
        };
        Ok(self
            .store
            .load_user_by_id(&user_id)
            .await
            .map_err(|error| map_store_error(error, "user identity"))?
            .map(|record| record.user)
            .filter(|user| user.enabled))
    }

    async fn verify_admin_api_key(&self, key: &str) -> Result<bool, AdminError> {
        if !valid_admin_api_key_shape(key) {
            return Ok(false);
        }
        let stored = self
            .store
            .load_admin_api_key()
            .await
            .map_err(|error| map_store_error(error, "administrator API key"))?;
        Ok(stored.as_ref().is_some_and(|stored| {
            let stored = stored.expose_for_auth();
            key.len() == stored.len() && bool::from(key.as_bytes().ct_eq(stored.as_bytes()))
        }))
    }

    async fn login(
        &self,
        command: LoginCommand,
        source_ip: IpAddr,
        previous_session_id: Option<&str>,
    ) -> Result<LoginResult, LoginError> {
        // 便捷入口不绕过登录防护；启用Turnstile时无证明必须拒绝。
        self.login_inner(
            command,
            source_ip,
            previous_session_id,
            LoginProtectionProof {
                token: None,
                remote_ip: Some(source_ip),
            },
        )
        .await
    }

    async fn login_with_protection(
        &self,
        command: LoginCommand,
        protection: LoginProtectionProof,
        previous_session_id: Option<&str>,
    ) -> Result<LoginResult, LoginError> {
        let source_ip = protection.remote_ip.ok_or(LoginError::Unavailable)?;
        self.login_inner(command, source_ip, previous_session_id, protection)
            .await
    }

    async fn logout(&self, session_id: &str) -> Result<(), AdminError> {
        let session = self
            .store
            .delete_session(session_id)
            .await
            .map_err(|error| map_store_error(error, "authentication session"))?;
        if let Some(AuthSession {
            subject: SessionSubject::Admin { admin_user_id, .. },
            ..
        }) = session
        {
            self.store
                .append_audit_event(self.auth_audit("admin.logout", &admin_user_id))
                .await
                .map_err(|error| map_store_error(error, "administrator audit"))?;
        }
        Ok(())
    }

    async fn public_auth_settings(&self) -> Result<PublicAuthSettings, AdminError> {
        let settings = self
            .store
            .load_turnstile_settings()
            .await
            .map_err(|error| map_store_error(error, "auth settings"))?;
        Ok(PublicAuthSettings {
            turnstile_enabled: settings.enabled,
            turnstile_site_key: settings.site_key,
        })
    }
}

fn session_ttl(minutes: u64) -> Duration {
    Duration::minutes(
        i64::try_from(minutes)
            .unwrap_or(MAX_SESSION_TTL_MINUTES)
            .clamp(1, MAX_SESSION_TTL_MINUTES),
    )
}

fn hash_admin_password(password: &str) -> Result<String, AdminError> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|hash| hash.to_string())
        .map_err(|_| AdminError::internal("管理员密码哈希失败"))
}

fn verify_admin_password(password: &str, encoded: &str) -> Result<bool, AdminError> {
    let hash = PasswordHash::new(encoded)
        .map_err(|_| AdminError::internal("已保存的管理员密码哈希不合法"))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &hash)
        .is_ok())
}

fn random_session_token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    format!("session_{}", URL_SAFE_NO_PAD.encode(bytes))
}

fn valid_admin_api_key_shape(value: &str) -> bool {
    value.len() == 70
        && value.starts_with("admin-")
        && value[6..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

// 指纹只绑定已加盐的密码哈希，不把密码或原始哈希复制到 Redis 会话。
fn password_fingerprint(password_hash: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(password_hash.as_bytes()))
}

pub(super) fn session_fingerprint(password_hash: &str, session_version: u64) -> String {
    format!("{}:{session_version}", password_fingerprint(password_hash))
}

fn session_fingerprint_matches(
    fingerprint: &str,
    password_hash: &str,
    session_version: u64,
) -> bool {
    fingerprint == session_fingerprint(password_hash, session_version)
}

pub(super) fn session_user_binding(subject: &SessionSubject) -> Option<(&str, UserRole, &str)> {
    match subject {
        SessionSubject::Admin {
            admin_user_id,
            credential_fingerprint,
        } => Some((admin_user_id, UserRole::Admin, credential_fingerprint)),
        SessionSubject::User {
            user_id,
            credential_fingerprint,
        } => Some((user_id, UserRole::User, credential_fingerprint)),
        SessionSubject::Key { .. } => None,
    }
}
