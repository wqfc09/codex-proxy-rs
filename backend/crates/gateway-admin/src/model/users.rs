//! 普通用户、统一会话与登录防护的领域事实。

use std::{fmt, net::IpAddr};

use chrono::{DateTime, Utc};
use gateway_core::policy::UserRateLimits;

pub const MAX_SESSION_TTL_MINUTES: u64 = 366 * 24 * 60;

/// 浏览器用户角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserRole {
    Admin,
    User,
}

impl UserRole {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::User => "user",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "admin" => Some(Self::Admin),
            "user" => Some(Self::User),
            _ => None,
        }
    }
}

/// 不包含密码哈希的用户公开领域记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRecord {
    pub id: String,
    pub username: String,
    pub role: UserRole,
    pub enabled: bool,
    /// 账户级准入限制：`None` 表示不限，`Some(0)` 表示实际零。
    pub limits: UserRateLimits,
    pub session_version: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 由管理员维护的账户默认上游身份；None 表示继承系统，owned Key 可再显式覆盖单个 Provider。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UserKeyIdentity {
    pub openai: Option<gateway_core::account::OpaqueProviderData>,
    pub xai: Option<gateway_core::account::OpaqueProviderData>,
}

/// Store 返回给认证用例的凭据记录；Debug 永不暴露哈希。
#[derive(Clone, PartialEq, Eq)]
pub struct UserCredentialRecord {
    pub user: UserRecord,
    pub password_hash: String,
}

impl fmt::Debug for UserCredentialRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UserCredentialRecord")
            .field("user", &self.user)
            .field("password_hash", &"[REDACTED]")
            .finish()
    }
}

/// 登录时可选的人机验证证明。
#[derive(Clone, PartialEq, Eq)]
pub struct LoginProtectionProof {
    pub token: Option<String>,
    pub remote_ip: Option<IpAddr>,
}

impl fmt::Debug for LoginProtectionProof {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LoginProtectionProof")
            .field("token", &self.token.as_ref().map(|_| "[REDACTED]"))
            .field("remote_ip", &self.remote_ip)
            .finish()
    }
}

/// 浏览器会话有效期 override；未持久化时继续使用启动配置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionTtlSettings {
    pub admin_minutes: u64,
    pub user_minutes: u64,
}

impl SessionTtlSettings {
    #[must_use]
    pub const fn new(admin_minutes: u64, user_minutes: u64) -> Self {
        Self {
            admin_minutes,
            user_minutes,
        }
    }
}

/// Turnstile 私密配置；secret 只能用于验证或显式持久化。
#[derive(Clone, PartialEq, Eq)]
pub struct TurnstileSettings {
    pub enabled: bool,
    pub site_key: Option<String>,
    secret_key: Option<String>,
}

impl TurnstileSettings {
    #[must_use]
    pub fn new(enabled: bool, site_key: Option<String>, secret_key: Option<String>) -> Self {
        Self {
            enabled,
            site_key,
            secret_key,
        }
    }

    #[must_use]
    pub fn secret_for_verification(&self) -> Option<&str> {
        self.secret_key.as_deref()
    }

    #[must_use]
    pub fn secret_for_store(&self) -> Option<&str> {
        self.secret_key.as_deref()
    }

    #[must_use]
    pub fn has_secret(&self) -> bool {
        self.secret_key.is_some()
    }
}

impl Default for TurnstileSettings {
    fn default() -> Self {
        Self::new(false, None, None)
    }
}

impl fmt::Debug for TurnstileSettings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TurnstileSettings")
            .field("enabled", &self.enabled)
            .field("site_key", &self.site_key)
            .field(
                "secret_key",
                &self.secret_key.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

/// 未认证登录页可读取的公开防护配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicAuthSettings {
    pub turnstile_enabled: bool,
    pub turnstile_site_key: Option<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct CreateUser {
    pub username: String,
    pub password: String,
    pub role: UserRole,
    pub enabled: bool,
}

impl fmt::Debug for CreateUser {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreateUser")
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .field("enabled", &self.enabled)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateUser {
    pub user_id: String,
    pub username: Option<String>,
    pub role: Option<UserRole>,
    pub enabled: Option<bool>,
    /// `None` 表示本次不修改；内层 `None` 表示显式设置为不限。
    pub max_concurrency: Option<Option<u64>>,
    pub requests_per_minute: Option<Option<u64>>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ResetUserPassword {
    pub user_id: String,
    pub password: String,
}

impl fmt::Debug for ResetUserPassword {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResetUserPassword")
            .field("user_id", &self.user_id)
            .field("password", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct UpdateOwnUsername {
    pub user_id: String,
    pub current_session_id: String,
    pub current_password: String,
    pub username: String,
}

impl fmt::Debug for UpdateOwnUsername {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UpdateOwnUsername")
            .field("user_id", &self.user_id)
            .field("current_password", &"[REDACTED]")
            .field("username", &self.username)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct UpdateOwnPassword {
    pub user_id: String,
    pub current_session_id: String,
    pub current_password: String,
    pub new_password: String,
}

impl fmt::Debug for UpdateOwnPassword {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UpdateOwnPassword")
            .field("user_id", &self.user_id)
            .field("current_session_id", &"[REDACTED]")
            .field("current_password", &"[REDACTED]")
            .field("new_password", &"[REDACTED]")
            .finish()
    }
}
