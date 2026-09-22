//! 用户管理、个人资料与登录防护设置用例。

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use gateway_core::{policy::UserRateLimits, runtime::SnapshotControl};
use uuid::Uuid;

use crate::{
    model::{
        AdminError, AdminErrorKind, MutationContext,
        users::{
            CreateUser, MAX_SESSION_TTL_MINUTES, ResetUserPassword, SessionTtlSettings,
            TurnstileSettings, UpdateOwnPassword, UpdateOwnUsername, UpdateUser,
            UserCredentialRecord, UserRecord, UserRole,
        },
    },
    ports::store::AuthStore,
};

use super::{
    map_store_error,
    password::{hash_password, validate_username, verify_password},
    publish_committed,
};

#[async_trait]
pub trait UsersService: Send + Sync {
    async fn key_identity(
        &self,
        user_id: &str,
    ) -> Result<crate::model::users::UserKeyIdentity, AdminError>;
    async fn replace_key_identity(
        &self,
        context: &MutationContext,
        user_id: &str,
        identity: crate::model::users::UserKeyIdentity,
    ) -> Result<(), AdminError>;
    async fn list(&self) -> Result<Vec<UserRecord>, AdminError>;
    async fn create(
        &self,
        context: &MutationContext,
        command: CreateUser,
    ) -> Result<UserRecord, AdminError>;
    async fn update(
        &self,
        context: &MutationContext,
        command: UpdateUser,
    ) -> Result<UserRecord, AdminError>;
    async fn reset_password(
        &self,
        context: &MutationContext,
        command: ResetUserPassword,
    ) -> Result<(), AdminError>;
    async fn delete(&self, context: &MutationContext, user_id: &str) -> Result<(), AdminError>;
    async fn revoke_sessions(
        &self,
        context: &MutationContext,
        user_id: &str,
    ) -> Result<(), AdminError>;
    async fn me(&self, user_id: &str) -> Result<UserRecord, AdminError>;
    async fn update_own_username(
        &self,
        command: UpdateOwnUsername,
    ) -> Result<UserRecord, AdminError>;
    async fn update_own_password(&self, command: UpdateOwnPassword) -> Result<(), AdminError>;
    async fn session_ttl_settings(&self) -> Result<SessionTtlSettings, AdminError>;
    async fn replace_session_ttl_settings(
        &self,
        context: &MutationContext,
        settings: SessionTtlSettings,
    ) -> Result<SessionTtlSettings, AdminError>;
    async fn turnstile_settings(&self) -> Result<TurnstileSettings, AdminError>;
    async fn replace_turnstile_settings(
        &self,
        context: &MutationContext,
        enabled: bool,
        site_key: Option<String>,
        secret_key: Option<String>,
    ) -> Result<TurnstileSettings, AdminError>;
}

pub(crate) struct DefaultUsersService {
    providers: crate::ports::provider::ProviderAdminRegistry,
    store: Arc<dyn AuthStore>,
    snapshot: Arc<dyn SnapshotControl>,
    default_session_ttl: SessionTtlSettings,
}

impl DefaultUsersService {
    #[must_use]
    pub(crate) fn new(
        store: Arc<dyn AuthStore>,
        snapshot: Arc<dyn SnapshotControl>,
        default_admin_session_ttl_minutes: u64,
        default_user_session_ttl_minutes: u64,
        providers: crate::ports::provider::ProviderAdminRegistry,
    ) -> Self {
        Self {
            providers,
            store,
            snapshot,
            default_session_ttl: SessionTtlSettings::new(
                default_admin_session_ttl_minutes.clamp(1, MAX_SESSION_TTL_MINUTES),
                default_user_session_ttl_minutes.clamp(1, MAX_SESSION_TTL_MINUTES),
            ),
        }
    }

    async fn credential(&self, user_id: &str) -> Result<UserCredentialRecord, AdminError> {
        self.store
            .load_user_by_id(user_id)
            .await
            .map_err(|error| map_store_error(error, "user identity"))?
            .ok_or_else(|| AdminError::not_found("用户不存在"))
    }

    async fn managed_user(&self, user_id: &str) -> Result<UserCredentialRecord, AdminError> {
        self.credential(user_id).await
    }

    async fn current_account_session(
        &self,
        session_id: &str,
        current: &UserCredentialRecord,
    ) -> Result<crate::model::auth::AuthSession, AdminError> {
        let session = self
            .store
            .load_session(session_id)
            .await
            .map_err(|error| map_store_error(error, "user session"))?
            .ok_or_else(|| AdminError::new(AdminErrorKind::Unauthorized, "当前会话已失效"))?;
        let valid = super::auth::session_user_binding(&session.subject).is_some_and(
            |(id, role, fingerprint)| {
                id == current.user.id
                    && role == current.user.role
                    && fingerprint
                        == super::auth::session_fingerprint(
                            &current.password_hash,
                            current.user.session_version,
                        )
            },
        ) && session.expires_at > Utc::now();
        if !valid {
            return Err(AdminError::new(
                AdminErrorKind::Unauthorized,
                "当前会话已失效",
            ));
        }
        Ok(session)
    }
}

#[async_trait]
impl UsersService for DefaultUsersService {
    async fn key_identity(
        &self,
        user_id: &str,
    ) -> Result<crate::model::users::UserKeyIdentity, AdminError> {
        self.managed_user(user_id).await?;
        self.store
            .load_user_key_identity(user_id)
            .await
            .map_err(|error| map_store_error(error, "user key identity"))
    }

    async fn replace_key_identity(
        &self,
        context: &MutationContext,
        user_id: &str,
        mut identity: crate::model::users::UserKeyIdentity,
    ) -> Result<(), AdminError> {
        self.managed_user(user_id).await?;
        if identity
            .openai
            .as_ref()
            .is_some_and(|profile| profile.expose_to_provider().is_empty())
        {
            identity.openai = None;
        }
        if identity
            .xai
            .as_ref()
            .is_some_and(|profile| profile.expose_to_provider().is_empty())
        {
            identity.xai = None;
        }
        // Provider 继续拥有各自的身份语义与校验；User 只保存选择并发布快照。
        for (provider, profile) in [("openai", &identity.openai), ("xai", &identity.xai)] {
            if let Some(profile) = profile {
                let kind = gateway_core::routing::ProviderKind::new(provider)
                    .map_err(|_| AdminError::invalid("Provider 不合法"))?;
                self.providers
                    .require(&kind)
                    .and_then(|provider| provider.preview_client_profile(profile))
                    .map_err(|error| super::map_provider_error(error, "client profile"))?;
            }
        }
        let revision = self
            .store
            .replace_user_key_identity(user_id, identity, context)
            .await
            .map_err(|error| map_store_error(error, "user key identity"))?;
        publish_committed(self.snapshot.as_ref(), revision).await
    }

    async fn list(&self) -> Result<Vec<UserRecord>, AdminError> {
        self.store
            .list_users()
            .await
            .map_err(|error| map_store_error(error, "user identity"))
    }

    async fn create(
        &self,
        context: &MutationContext,
        command: CreateUser,
    ) -> Result<UserRecord, AdminError> {
        if command.role == UserRole::Admin {
            return Err(AdminError::new(
                AdminErrorKind::Forbidden,
                "只能通过启动配置初始化管理员",
            ));
        }
        validate_username(&command.username)?;
        let password_hash = hash_password(&command.password)?;
        let now = Utc::now();
        let user = UserCredentialRecord {
            user: UserRecord {
                id: format!("user_{}", Uuid::now_v7().simple()),
                username: command.username,
                role: command.role,
                enabled: command.enabled,
                limits: UserRateLimits {
                    max_concurrency: Some(0),
                    requests_per_minute: Some(0),
                },
                session_version: 1,
                created_at: now,
                updated_at: now,
            },
            password_hash,
        };
        self.store
            .create_user(user, context)
            .await
            .map_err(|error| map_store_error(error, "user identity"))
    }

    async fn update(
        &self,
        context: &MutationContext,
        command: UpdateUser,
    ) -> Result<UserRecord, AdminError> {
        let current = self.managed_user(&command.user_id).await?;
        if let Some(username) = command.username.as_deref() {
            validate_username(username)?;
        }
        let removes_enabled_admin = current.user.role == UserRole::Admin
            && current.user.enabled
            && (command.role == Some(UserRole::User) || command.enabled == Some(false));
        if removes_enabled_admin {
            let users = self
                .store
                .list_users()
                .await
                .map_err(|error| map_store_error(error, "user identity"))?;
            let has_other_enabled_admin = users.iter().any(|user| {
                user.id != command.user_id && user.enabled && user.role == UserRole::Admin
            });
            if !has_other_enabled_admin {
                return Err(AdminError::new(
                    AdminErrorKind::Forbidden,
                    "至少需要保留一个启用的管理员账户",
                ));
            }
        }
        let (config_revision, user) = self
            .store
            .update_user(&command, context)
            .await
            .map_err(|error| map_store_error(error, "user identity"))?;
        publish_committed(self.snapshot.as_ref(), config_revision).await?;
        // 禁用或改角色会在 users 表内递增 session_version；认证恢复时回查该事实并拒绝旧会话。
        Ok(user)
    }

    async fn reset_password(
        &self,
        context: &MutationContext,
        command: ResetUserPassword,
    ) -> Result<(), AdminError> {
        let current = self.managed_user(&command.user_id).await?;
        if verify_password(&command.password, &current.password_hash)? {
            return Err(AdminError::invalid("新密码不能与当前密码相同"));
        }
        let password_hash = hash_password(&command.password)?;
        let _updated = self
            .store
            .reset_user_password_hash(&command.user_id, &password_hash, context)
            .await
            .map_err(|error| map_store_error(error, "user password"))?;
        Ok(())
    }

    async fn delete(&self, context: &MutationContext, user_id: &str) -> Result<(), AdminError> {
        if matches!(
            &context.actor,
            crate::model::MutationActor::AdminSession { admin_user_id }
                if admin_user_id == user_id
        ) {
            return Err(AdminError::new(
                AdminErrorKind::Forbidden,
                "当前登录管理员不能删除自己的账户",
            ));
        }
        let current = self.managed_user(user_id).await?;
        if current.user.role == UserRole::Admin && current.user.enabled {
            let users = self
                .store
                .list_users()
                .await
                .map_err(|error| map_store_error(error, "user identity"))?;
            let has_other_enabled_admin = users
                .iter()
                .any(|user| user.id != user_id && user.enabled && user.role == UserRole::Admin);
            if !has_other_enabled_admin {
                return Err(AdminError::new(
                    AdminErrorKind::Forbidden,
                    "至少需要保留一个启用的管理员账户",
                ));
            }
        }
        let revision = self
            .store
            .delete_user(user_id, context)
            .await
            .map_err(|error| map_store_error(error, "user identity"))?;
        publish_committed(self.snapshot.as_ref(), revision).await?;
        Ok(())
    }

    async fn revoke_sessions(
        &self,
        context: &MutationContext,
        user_id: &str,
    ) -> Result<(), AdminError> {
        self.managed_user(user_id).await?;
        self.store
            .bump_user_session_version(user_id, context)
            .await
            .map_err(|error| map_store_error(error, "user sessions"))?;
        // 版本递增使全部旧会话失效；无法可靠统计 Redis 中实际失效的会话数量。
        Ok(())
    }

    async fn me(&self, user_id: &str) -> Result<UserRecord, AdminError> {
        let user = self.credential(user_id).await?;
        if !user.user.enabled {
            return Err(AdminError::new(
                AdminErrorKind::Unauthorized,
                "用户已被禁用",
            ));
        }
        Ok(user.user)
    }

    async fn update_own_username(
        &self,
        command: UpdateOwnUsername,
    ) -> Result<UserRecord, AdminError> {
        validate_username(&command.username)?;
        let current = self.credential(&command.user_id).await?;
        if !current.user.enabled
            || !verify_password(&command.current_password, &current.password_hash)?
        {
            return Err(AdminError::new(
                AdminErrorKind::Unauthorized,
                "当前密码不正确",
            ));
        }
        self.current_account_session(&command.current_session_id, &current)
            .await?;
        self.store
            .update_user_username(
                &command.user_id,
                &command.username,
                current.user.session_version,
            )
            .await
            .map_err(|error| map_store_error(error, "user identity"))?
            .ok_or_else(|| AdminError::new(AdminErrorKind::Unauthorized, "当前会话已失效"))
    }

    async fn update_own_password(&self, command: UpdateOwnPassword) -> Result<(), AdminError> {
        let current = self.credential(&command.user_id).await?;
        if !current.user.enabled
            || !verify_password(&command.current_password, &current.password_hash)?
        {
            return Err(AdminError::new(
                AdminErrorKind::Unauthorized,
                "当前密码不正确",
            ));
        }
        let current_session = self
            .current_account_session(&command.current_session_id, &current)
            .await?;
        if command.new_password == command.current_password {
            return Err(AdminError::invalid("新密码不能与当前密码相同"));
        }
        let password_hash = hash_password(&command.new_password)?;
        let _updated = self
            .store
            .update_user_password_hash_if_matches(
                &command.user_id,
                &current.password_hash,
                &password_hash,
            )
            .await
            .map_err(|error| map_store_error(error, "user password"))?;
        if !_updated {
            return Err(AdminError::new(
                AdminErrorKind::Conflict,
                "密码已被其他操作修改，请重新验证后再试",
            ));
        }
        let next_version = current
            .user
            .session_version
            .checked_add(1)
            .ok_or_else(|| AdminError::new(AdminErrorKind::Unavailable, "会话版本不可用"))?;
        let mut replacement = current_session.clone();
        match &mut replacement.subject {
            crate::model::auth::SessionSubject::Admin {
                credential_fingerprint,
                ..
            }
            | crate::model::auth::SessionSubject::User {
                credential_fingerprint,
                ..
            } => {
                *credential_fingerprint =
                    super::auth::session_fingerprint(&password_hash, next_version);
            }
            crate::model::auth::SessionSubject::Key { .. } => {
                return Err(AdminError::new(
                    AdminErrorKind::Unauthorized,
                    "当前会话不是账户会话",
                ));
            }
        }
        // 仅刷新发起改密的旧会话，保持原到期时间；并发注销/轮换后不重新创建。
        // 使用预期版本而非重新读取最新版本，不能越过并发角色变更或显式撤销。
        if !self
            .store
            .replace_session_if_matches(&command.current_session_id, &current_session, &replacement)
            .await
            .map_err(|error| map_store_error(error, "user session"))?
        {
            return Err(AdminError::new(
                AdminErrorKind::Unauthorized,
                "密码已更新，当前会话已失效，请重新登录",
            ));
        }
        Ok(())
    }

    async fn session_ttl_settings(&self) -> Result<SessionTtlSettings, AdminError> {
        self.store
            .load_session_ttl_settings()
            .await
            .map_err(|error| map_store_error(error, "auth settings"))
            .map(|settings| settings.unwrap_or(self.default_session_ttl))
    }

    async fn replace_session_ttl_settings(
        &self,
        context: &MutationContext,
        settings: SessionTtlSettings,
    ) -> Result<SessionTtlSettings, AdminError> {
        if settings.admin_minutes == 0
            || settings.user_minutes == 0
            || settings.admin_minutes > MAX_SESSION_TTL_MINUTES
            || settings.user_minutes > MAX_SESSION_TTL_MINUTES
        {
            return Err(AdminError::invalid(
                "Session TTL 必须在 1 分钟到 366 天之间",
            ));
        }
        self.store
            .replace_session_ttl_settings(settings, context)
            .await
            .map_err(|error| map_store_error(error, "auth settings"))
    }

    async fn turnstile_settings(&self) -> Result<TurnstileSettings, AdminError> {
        self.store
            .load_turnstile_settings()
            .await
            .map_err(|error| map_store_error(error, "auth settings"))
    }

    async fn replace_turnstile_settings(
        &self,
        context: &MutationContext,
        enabled: bool,
        site_key: Option<String>,
        secret_key: Option<String>,
    ) -> Result<TurnstileSettings, AdminError> {
        let current = self.turnstile_settings().await?;
        let current_secret = current.secret_for_verification().map(str::to_owned);
        let site_key = site_key
            .or(current.site_key)
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let secret_key = secret_key
            .or(current_secret)
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        if enabled && (site_key.is_none() || secret_key.is_none()) {
            return Err(AdminError::invalid(
                "启用 Turnstile 前必须配置 site key 和 secret key",
            ));
        }
        self.store
            .replace_turnstile_settings(
                TurnstileSettings::new(enabled, site_key, secret_key),
                context,
            )
            .await
            .map_err(|error| map_store_error(error, "auth settings"))
    }
}
