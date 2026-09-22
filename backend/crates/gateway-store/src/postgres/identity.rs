//! 统一用户身份与登录防护设置的 PostgreSQL owner。

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};

use crate::{ConflictKind, Revision, StoreBackend, StoreError, StoreResult, postgres_unavailable};

use super::{
    AdminAuditEvent, append_admin_audit_event_in_transaction, bump_config_revision_in_transaction,
};

#[derive(Clone, PartialEq, Eq)]
pub struct StoredUser {
    pub id: String,
    pub username: String,
    pub password_hash: String,
    pub role: String,
    pub enabled: bool,
    pub max_concurrency: Option<i64>,
    pub requests_per_minute: Option<i64>,
    pub session_version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl std::fmt::Debug for StoredUser {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StoredUser")
            .field("id", &self.id)
            .field("username", &self.username)
            .field("password_hash", &"[REDACTED]")
            .field("role", &self.role)
            .field("enabled", &self.enabled)
            .field("max_concurrency", &self.max_concurrency)
            .field("requests_per_minute", &self.requests_per_minute)
            .field("session_version", &self.session_version)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StoredUserUpdate<'a> {
    pub id: &'a str,
    pub username: Option<&'a str>,
    pub role: Option<&'a str>,
    pub enabled: Option<bool>,
    pub max_concurrency: Option<Option<u64>>,
    pub requests_per_minute: Option<Option<u64>>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct StoredAuthSettings {
    pub turnstile_enabled: bool,
    pub turnstile_site_key: Option<String>,
    pub turnstile_secret_key: Option<String>,
}

impl std::fmt::Debug for StoredAuthSettings {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StoredAuthSettings")
            .field("turnstile_enabled", &self.turnstile_enabled)
            .field("turnstile_site_key", &self.turnstile_site_key)
            .field(
                "turnstile_secret_key",
                &self.turnstile_secret_key.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

#[derive(Clone)]
pub struct PgIdentityRepository {
    pool: PgPool,
}

impl PgIdentityRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn ensure_default_admin(&self, id: &str, password_hash: &str) -> StoreResult<bool> {
        validate_text("user", "id", id, 128)?;
        validate_text("user", "password_hash", password_hash, 1024)?;
        let mut transaction = begin(&self.pool, "begin default admin bootstrap").await?;
        sqlx::query(
            "insert into admin_users (id, password_hash, created_at, updated_at)
             values ($1, $2, now(), now())
             on conflict (id) do nothing",
        )
        .bind(id)
        .bind(password_hash)
        .execute(&mut *transaction)
        .await
        .map_err(|_| postgres_unavailable("ensure legacy admin user"))?;
        let result = sqlx::query(
            "insert into users (id, username, password_hash, role, enabled,
                 max_concurrency_override, requests_per_minute_override, created_at, updated_at)
             values ($1, $1, $2, 'admin', true, null, null, now(), now())
             on conflict (id) do nothing",
        )
        .bind(id)
        .bind(password_hash)
        .execute(&mut *transaction)
        .await
        .map_err(|error| map_user_write_error(id, error, "ensure default admin"))?;
        transaction
            .commit()
            .await
            .map_err(|_| postgres_unavailable("commit default admin bootstrap"))?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn key_identity(
        &self,
        user_id: &str,
    ) -> StoreResult<gateway_admin::model::users::UserKeyIdentity> {
        use gateway_core::account::OpaqueProviderData;
        let value = sqlx::query_scalar::<
            _,
            sqlx::types::Json<serde_json::Map<String, serde_json::Value>>,
        >("select provider_request_profiles_json from users where id=$1")
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| postgres_unavailable("load user key identity"))?
        .ok_or_else(|| StoreError::NotFound {
            entity: "user",
            id: user_id.to_owned(),
        })?
        .0;
        let profile = |name: &str| -> StoreResult<Option<OpaqueProviderData>> {
            let Some(value) = value.get(name) else {
                return Ok(None);
            };
            let object = value
                .as_object()
                .cloned()
                .ok_or_else(|| StoreError::InvalidData {
                    entity: "user key identity",
                    message: "profile must be an object".to_owned(),
                })?;
            Ok((!object.is_empty()).then(|| OpaqueProviderData::new(object)))
        };
        Ok(gateway_admin::model::users::UserKeyIdentity {
            openai: profile("openai")?,
            xai: profile("xai")?,
        })
    }

    pub async fn replace_key_identity(
        &self,
        user_id: &str,
        identity: gateway_admin::model::users::UserKeyIdentity,
        audit: AdminAuditEvent,
    ) -> StoreResult<Revision> {
        let mut profiles = serde_json::Map::new();
        for (name, profile) in [("openai", identity.openai), ("xai", identity.xai)] {
            if let Some(profile) = profile {
                let value = profile.into_inner();
                if !value.is_empty() {
                    profiles.insert(name.to_owned(), serde_json::Value::Object(value));
                }
            }
        }
        let mut transaction = begin(&self.pool, "begin user key identity mutation").await?;
        // 与用户、分组和 Key 写入保持 revision -> User 的锁顺序。
        let revision = bump_config_revision_in_transaction(&mut transaction).await?;
        let changed = sqlx::query(
            "update users set provider_request_profiles_json=$2,
            updated_at=greatest(clock_timestamp(),updated_at) where id=$1",
        )
        .bind(user_id)
        .bind(sqlx::types::Json(profiles))
        .execute(&mut *transaction)
        .await
        .map_err(|error| map_user_write_error(user_id, error, "update user key identity"))?;
        if changed.rows_affected() != 1 {
            return Err(StoreError::NotFound {
                entity: "user",
                id: user_id.to_owned(),
            });
        }
        append_admin_audit_event_in_transaction(&mut transaction, audit, revision).await?;
        commit(transaction, "commit user key identity").await?;
        Ok(revision)
    }

    pub async fn user_by_username(&self, username: &str) -> StoreResult<Option<StoredUser>> {
        validate_text("user", "username", username, 128)?;
        fetch_user(
            &self.pool,
            "select id, username, password_hash, role, enabled, max_concurrency_override, requests_per_minute_override, session_version, created_at, updated_at
             from users where username = $1",
            username,
        )
        .await
    }

    pub async fn user_by_id(&self, id: &str) -> StoreResult<Option<StoredUser>> {
        validate_text("user", "id", id, 128)?;
        fetch_user(
            &self.pool,
            "select id, username, password_hash, role, enabled, max_concurrency_override, requests_per_minute_override, session_version, created_at, updated_at
             from users where id = $1",
            id,
        )
        .await
    }

    pub async fn list_users(&self) -> StoreResult<Vec<StoredUser>> {
        sqlx::query_as::<_, UserRow>(
            "select id, username, password_hash, role, enabled, max_concurrency_override, requests_per_minute_override, session_version, created_at, updated_at
             from users order by created_at asc, id asc",
        )
        .fetch_all(&self.pool)
        .await
        .map(|rows| rows.into_iter().map(StoredUser::from).collect())
        .map_err(|_| postgres_unavailable("list users"))
    }

    pub async fn create_user(&self, user: &StoredUser, audit: AdminAuditEvent) -> StoreResult<()> {
        validate_user(user)?;
        let mut transaction = begin(&self.pool, "begin create user").await?;
        sqlx::query(
            "insert into users (
               id, username, password_hash, role, enabled, max_concurrency_override,
               requests_per_minute_override, session_version, created_at, updated_at
             ) values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(&user.id)
        .bind(&user.username)
        .bind(&user.password_hash)
        .bind(&user.role)
        .bind(user.enabled)
        .bind(user.max_concurrency)
        .bind(user.requests_per_minute)
        .bind(user.session_version)
        .bind(user.created_at)
        .bind(user.updated_at)
        .execute(&mut *transaction)
        .await
        .map_err(|error| map_user_write_error(&user.id, error, "create user"))?;
        if user.role == "admin" {
            ensure_admin_audit_identity_in_transaction(&mut transaction, &user.id).await?;
        }
        append_admin_audit_event_in_transaction_without_revision(&mut transaction, audit).await?;
        commit(transaction, "commit create user").await
    }

    /// 旧审计外键仍指向 admin_users；仅从已认证的 canonical Admin 补齐缺失投影。
    pub async fn ensure_admin_audit_identity(&self, id: &str) -> StoreResult<()> {
        let mut tx = begin(&self.pool, "begin admin audit identity").await?;
        ensure_admin_audit_identity_in_transaction(&mut tx, id).await?;
        commit(tx, "commit admin audit identity").await
    }

    pub async fn update_user(
        &self,
        command: StoredUserUpdate<'_>,
        audit: AdminAuditEvent,
    ) -> StoreResult<(Revision, StoredUser)> {
        let StoredUserUpdate {
            id,
            username,
            role,
            enabled,
            max_concurrency,
            requests_per_minute,
        } = command;
        validate_text("user", "id", id, 128)?;
        if let Some(username) = username {
            validate_text("user", "username", username, 128)?;
        }
        if let Some(role) = role
            && !matches!(role, "admin" | "user")
        {
            return Err(StoreError::InvalidData {
                entity: "user",
                message: "role must be admin or user".to_owned(),
            });
        }
        let max_concurrency_was_explicit = max_concurrency.is_some();
        let requests_per_minute_was_explicit = requests_per_minute.is_some();
        let max_concurrency = max_concurrency
            .flatten()
            .map(|value| i64::try_from(value).map_err(|_| invalid_limit("max_concurrency")))
            .transpose()?;
        let requests_per_minute = requests_per_minute
            .flatten()
            .map(|value| i64::try_from(value).map_err(|_| invalid_limit("requests_per_minute")))
            .transpose()?;
        let mut transaction = begin(&self.pool, "begin update user").await?;
        // 与其他管理模块先锁 revision，再以同一用户行锁串行化授权和计费变更。
        sqlx::query("select config_revision from runtime_settings where id=1 for update")
            .execute(&mut *transaction)
            .await
            .map_err(|_| postgres_unavailable("lock user update revision"))?;
        let (current_role, current_enabled) = sqlx::query_as::<_, (String, bool)>(
            "select role, enabled from users where id = $1 for update",
        )
        .bind(id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| postgres_unavailable("lock user for update"))?
        .ok_or_else(|| StoreError::NotFound {
            entity: "user",
            id: id.to_owned(),
        })?;
        let next_role = role.unwrap_or(current_role.as_str());
        let next_enabled = enabled.unwrap_or(current_enabled);
        if current_role == "admin"
            && current_enabled
            && !(next_role == "admin" && next_enabled)
            && !has_other_enabled_admin(&mut transaction, id).await?
        {
            return Err(last_enabled_admin_conflict(id));
        }
        let revision = bump_config_revision_in_transaction(&mut transaction).await?;
        let row = sqlx::query_as::<_, UserRow>(
            "update users
             set username = coalesce($2, username),
                 role = coalesce($3, role),
                 enabled = coalesce($4, enabled),
                 max_concurrency_override = case when $7::boolean then $5::bigint else max_concurrency_override end,
                 requests_per_minute_override = case when $8::boolean then $6::bigint else requests_per_minute_override end,
                 session_version = case when $4 = false or ($3::text is not null and role is distinct from $3::text) then session_version + 1 else session_version end,
                 updated_at = greatest(clock_timestamp(), updated_at)
             where id = $1
             returning id, username, password_hash, role, enabled, max_concurrency_override, requests_per_minute_override, session_version, created_at, updated_at",
        )
        .bind(id)
        .bind(username)
        .bind(role)
        .bind(enabled)
        .bind(max_concurrency)
        .bind(requests_per_minute)
        .bind(max_concurrency_was_explicit)
        .bind(requests_per_minute_was_explicit)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| map_user_write_error(id, error, "update user"))?
        .ok_or_else(|| StoreError::NotFound {
            entity: "user",
            id: id.to_owned(),
        })?;
        if role == Some("admin") {
            ensure_admin_audit_identity_in_transaction(&mut transaction, id).await?;
        }
        append_admin_audit_event_in_transaction(&mut transaction, audit, revision).await?;
        commit(transaction, "commit update user").await?;
        Ok((revision, row.into()))
    }

    pub async fn update_username(
        &self,
        id: &str,
        username: &str,
        expected_session_version: u64,
    ) -> StoreResult<Option<StoredUser>> {
        validate_text("user", "id", id, 128)?;
        validate_text("user", "username", username, 128)?;
        let expected_session_version =
            i64::try_from(expected_session_version).map_err(|_| StoreError::InvalidData {
                entity: "user",
                message: "session version is invalid".to_owned(),
            })?;
        let mut transaction = begin(&self.pool, "begin self username update").await?;
        let row = sqlx::query_as::<_, UserRow>(
            "update users set username = $2, updated_at = greatest(clock_timestamp(), updated_at)
             where id = $1 and enabled and session_version = $3
             returning id, username, password_hash, role, enabled, max_concurrency_override, requests_per_minute_override, session_version, created_at, updated_at",
        )
        .bind(id)
        .bind(username)
        .bind(expected_session_version)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| map_user_write_error(id, error, "update username"))?;
        let Some(row) = row else {
            transaction
                .rollback()
                .await
                .map_err(|_| postgres_unavailable("rollback stale username update"))?;
            return Ok(None);
        };
        append_admin_audit_event_in_transaction_without_revision(
            &mut transaction,
            self_service_audit(id, "user.username.update", "username"),
        )
        .await?;
        commit(transaction, "commit self username update").await?;
        Ok(Some(row.into()))
    }

    /// 管理员自助改密：按 canonical users 哈希原子更新并递增会话版本。
    pub async fn change_password(
        &self,
        id: &str,
        expected_hash: &str,
        password_hash: &str,
        audit: AdminAuditEvent,
    ) -> StoreResult<bool> {
        validate_text("user", "id", id, 128)?;
        validate_text("user", "password_hash", password_hash, 1024)?;
        let mut transaction = begin(&self.pool, "begin user password change").await?;
        let changed = sqlx::query(
            "update users
             set password_hash = $3, session_version = session_version + 1,
                 updated_at = greatest(clock_timestamp(), updated_at)
             where id = $1 and password_hash = $2 and enabled",
        )
        .bind(id)
        .bind(expected_hash)
        .bind(password_hash)
        .execute(&mut *transaction)
        .await
        .map_err(|_| postgres_unavailable("change user password"))?
        .rows_affected()
            == 1;
        if !changed {
            return Ok(false);
        }
        append_admin_audit_event_in_transaction_without_revision(&mut transaction, audit).await?;
        commit(transaction, "commit user password change").await?;
        Ok(true)
    }

    /// 自助改密只在读取到的 canonical 哈希仍未变化时提交。
    /// 账户可能由应用时间创建；数据库时钟或事务起点不能令更新时间倒退。
    pub async fn update_password_hash_if_matches(
        &self,
        id: &str,
        expected_hash: &str,
        password_hash: &str,
    ) -> StoreResult<bool> {
        self.change_password(
            id,
            expected_hash,
            password_hash,
            self_service_audit(id, "user.password.change", "password"),
        )
        .await
    }

    pub async fn reset_password_hash(
        &self,
        id: &str,
        password_hash: &str,
        audit: AdminAuditEvent,
    ) -> StoreResult<StoredUser> {
        validate_text("user", "id", id, 128)?;
        validate_text("user", "password_hash", password_hash, 1024)?;
        let mut transaction = begin(&self.pool, "begin reset user password").await?;
        let row = sqlx::query_as::<_, UserRow>(
            "update users
             set password_hash = $2,
                 session_version = session_version + 1,
                 updated_at = greatest(clock_timestamp(), updated_at)
             where id = $1
             returning id, username, password_hash, role, enabled, max_concurrency_override, requests_per_minute_override, session_version, created_at, updated_at",
        )
        .bind(id)
        .bind(password_hash)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| postgres_unavailable("reset user password"))?
        .ok_or_else(|| StoreError::NotFound {
            entity: "user",
            id: id.to_owned(),
        })?;
        append_admin_audit_event_in_transaction_without_revision(&mut transaction, audit).await?;
        commit(transaction, "commit reset user password").await?;
        Ok(row.into())
    }

    pub async fn bump_session_version(
        &self,
        id: &str,
        audit: AdminAuditEvent,
    ) -> StoreResult<StoredUser> {
        validate_text("user", "id", id, 128)?;
        let mut transaction = begin(&self.pool, "begin revoke user sessions").await?;
        let row = sqlx::query_as::<_, UserRow>(
            "update users
             set session_version = session_version + 1,
                 updated_at = greatest(clock_timestamp(), updated_at)
             where id = $1
             returning id, username, password_hash, role, enabled, max_concurrency_override, requests_per_minute_override, session_version, created_at, updated_at",
        )
        .bind(id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| postgres_unavailable("revoke user sessions"))?
        .ok_or_else(|| StoreError::NotFound {
            entity: "user",
            id: id.to_owned(),
        })?;
        append_admin_audit_event_in_transaction_without_revision(&mut transaction, audit).await?;
        commit(transaction, "commit revoke user sessions").await?;
        Ok(row.into())
    }

    pub async fn delete_user(&self, id: &str, audit: AdminAuditEvent) -> StoreResult<Revision> {
        validate_text("user", "id", id, 128)?;
        let mut transaction = begin(&self.pool, "begin delete user").await?;
        // 与 update_user 使用相同锁顺序；revision 行同时串行化管理员存活性检查。
        sqlx::query("select config_revision from runtime_settings where id=1 for update")
            .execute(&mut *transaction)
            .await
            .map_err(|_| postgres_unavailable("lock user delete revision"))?;
        let (role, enabled) = sqlx::query_as::<_, (String, bool)>(
            "select role, enabled from users where id = $1 for update",
        )
        .bind(id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|_| postgres_unavailable("lock user for delete"))?
        .ok_or_else(|| StoreError::NotFound {
            entity: "user",
            id: id.to_owned(),
        })?;
        if role == "admin" && enabled && !has_other_enabled_admin(&mut transaction, id).await? {
            return Err(last_enabled_admin_conflict(id));
        }
        sqlx::query("delete from users where id = $1")
            .bind(id)
            .execute(&mut *transaction)
            .await
            .map_err(|_| postgres_unavailable("delete user"))?;
        let revision = bump_config_revision_in_transaction(&mut transaction).await?;
        append_admin_audit_event_in_transaction(&mut transaction, audit, revision).await?;
        commit(transaction, "commit delete user").await?;
        Ok(revision)
    }

    pub async fn session_ttl_settings(&self) -> StoreResult<Option<(u64, u64)>> {
        let row = sqlx::query_as::<_, (Option<i64>, Option<i64>)>(
            "select admin_session_ttl_minutes, user_session_ttl_minutes
             from auth_settings where id = 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| postgres_unavailable("load session TTL settings"))?
        .ok_or_else(|| StoreError::NotFound {
            entity: "auth settings",
            id: "1".to_owned(),
        })?;
        match row {
            (None, None) => Ok(None),
            (Some(admin), Some(user)) => {
                let admin = u64::try_from(admin).map_err(|_| StoreError::InvalidData {
                    entity: "auth settings",
                    message: "admin session TTL is invalid".to_owned(),
                })?;
                let user = u64::try_from(user).map_err(|_| StoreError::InvalidData {
                    entity: "auth settings",
                    message: "user session TTL is invalid".to_owned(),
                })?;
                Ok(Some((admin, user)))
            }
            _ => Err(StoreError::InvalidData {
                entity: "auth settings",
                message: "session TTL override is incomplete".to_owned(),
            }),
        }
    }

    pub async fn replace_session_ttl_settings(
        &self,
        admin_minutes: u64,
        user_minutes: u64,
        audit: AdminAuditEvent,
    ) -> StoreResult<()> {
        let admin_minutes = i64::try_from(admin_minutes).map_err(|_| StoreError::InvalidData {
            entity: "auth settings",
            message: "admin session TTL is invalid".to_owned(),
        })?;
        let user_minutes = i64::try_from(user_minutes).map_err(|_| StoreError::InvalidData {
            entity: "auth settings",
            message: "user session TTL is invalid".to_owned(),
        })?;
        let mut transaction = begin(&self.pool, "begin update session TTL settings").await?;
        let result = sqlx::query(
            "update auth_settings
             set admin_session_ttl_minutes = $1,
                 user_session_ttl_minutes = $2,
                 updated_at = now()
             where id = 1",
        )
        .bind(admin_minutes)
        .bind(user_minutes)
        .execute(&mut *transaction)
        .await
        .map_err(|_| postgres_unavailable("update session TTL settings"))?;
        if result.rows_affected() == 0 {
            return Err(StoreError::NotFound {
                entity: "auth settings",
                id: "1".to_owned(),
            });
        }
        append_admin_audit_event_in_transaction_without_revision(&mut transaction, audit).await?;
        commit(transaction, "commit update session TTL settings").await
    }

    pub async fn auth_settings(&self) -> StoreResult<StoredAuthSettings> {
        let row = sqlx::query_as::<_, (bool, Option<String>, Option<String>)>(
            "select turnstile_enabled, turnstile_site_key, turnstile_secret_key
             from auth_settings where id = 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| postgres_unavailable("load auth settings"))?
        .ok_or_else(|| StoreError::NotFound {
            entity: "auth settings",
            id: "1".to_owned(),
        })?;
        Ok(StoredAuthSettings {
            turnstile_enabled: row.0,
            turnstile_site_key: row.1,
            turnstile_secret_key: row.2,
        })
    }

    pub async fn replace_auth_settings(
        &self,
        settings: &StoredAuthSettings,
        audit: AdminAuditEvent,
    ) -> StoreResult<()> {
        let mut transaction = begin(&self.pool, "begin update auth settings").await?;
        let result = sqlx::query(
            "update auth_settings
             set turnstile_enabled = $1,
                 turnstile_site_key = $2,
                 turnstile_secret_key = $3,
                 updated_at = now()
             where id = 1",
        )
        .bind(settings.turnstile_enabled)
        .bind(settings.turnstile_site_key.as_deref())
        .bind(settings.turnstile_secret_key.as_deref())
        .execute(&mut *transaction)
        .await
        .map_err(|_| postgres_unavailable("update auth settings"))?;
        if result.rows_affected() == 0 {
            return Err(StoreError::NotFound {
                entity: "auth settings",
                id: "1".to_owned(),
            });
        }
        append_admin_audit_event_in_transaction_without_revision(&mut transaction, audit).await?;
        commit(transaction, "commit update auth settings").await
    }
}

type UserRow = (
    String,
    String,
    String,
    String,
    bool,
    Option<i64>,
    Option<i64>,
    i64,
    DateTime<Utc>,
    DateTime<Utc>,
);

impl From<UserRow> for StoredUser {
    fn from(row: UserRow) -> Self {
        Self {
            id: row.0,
            username: row.1,
            password_hash: row.2,
            role: row.3,
            enabled: row.4,
            max_concurrency: row.5,
            requests_per_minute: row.6,
            session_version: row.7,
            created_at: row.8,
            updated_at: row.9,
        }
    }
}

async fn fetch_user(
    pool: &PgPool,
    sql: &'static str,
    value: &str,
) -> StoreResult<Option<StoredUser>> {
    sqlx::query_as::<_, UserRow>(sql)
        .bind(value)
        .fetch_optional(pool)
        .await
        .map(|row| row.map(Into::into))
        .map_err(|_| postgres_unavailable("read user"))
}

fn validate_user(user: &StoredUser) -> StoreResult<()> {
    validate_text("user", "id", &user.id, 128)?;
    validate_text("user", "username", &user.username, 128)?;
    validate_text("user", "password_hash", &user.password_hash, 1024)?;
    if !matches!(user.role.as_str(), "admin" | "user")
        || user.max_concurrency.is_some_and(|value| value < 0)
        || user.requests_per_minute.is_some_and(|value| value < 0)
        || user.session_version <= 0
        || user.created_at > user.updated_at
    {
        return Err(StoreError::InvalidData {
            entity: "user",
            message: "role or timestamps are invalid".to_owned(),
        });
    }
    Ok(())
}

async fn has_other_enabled_admin(
    transaction: &mut Transaction<'_, Postgres>,
    excluded_id: &str,
) -> StoreResult<bool> {
    sqlx::query_scalar::<_, bool>(
        "select exists(
           select 1 from users
           where id <> $1 and role = 'admin' and enabled = true
         )",
    )
    .bind(excluded_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(|_| postgres_unavailable("check remaining enabled administrator"))
}

fn last_enabled_admin_conflict(id: &str) -> StoreError {
    StoreError::Conflict {
        entity: "user",
        id: id.to_owned(),
        kind: ConflictKind::InvalidTransition,
    }
}

fn invalid_limit(field: &str) -> StoreError {
    StoreError::InvalidData {
        entity: "user",
        message: format!("{field} is outside the supported range"),
    }
}

fn validate_text(entity: &'static str, field: &str, value: &str, max: usize) -> StoreResult<()> {
    if value.is_empty() || value.len() > max || value.chars().any(char::is_control) {
        return Err(StoreError::InvalidData {
            entity,
            message: format!("{field} is invalid"),
        });
    }
    Ok(())
}

fn map_user_write_error(id: &str, error: sqlx::Error, operation: &'static str) -> StoreError {
    if error
        .as_database_error()
        .is_some_and(|database_error| database_error.is_unique_violation())
    {
        return StoreError::Conflict {
            entity: "user",
            id: id.to_owned(),
            kind: ConflictKind::DuplicateName,
        };
    }
    StoreError::Unavailable {
        backend: StoreBackend::PostgreSql,
        message: operation.to_owned(),
    }
}

async fn begin<'a>(
    pool: &'a PgPool,
    operation: &'static str,
) -> StoreResult<Transaction<'a, Postgres>> {
    pool.begin()
        .await
        .map_err(|_| postgres_unavailable(operation))
}

async fn commit(
    transaction: Transaction<'_, Postgres>,
    operation: &'static str,
) -> StoreResult<()> {
    transaction
        .commit()
        .await
        .map_err(|_| postgres_unavailable(operation))
}

async fn append_admin_audit_event_in_transaction_without_revision(
    transaction: &mut Transaction<'_, Postgres>,
    event: AdminAuditEvent,
) -> StoreResult<()> {
    event.validate()?;
    sqlx::query(
        "insert into admin_audit_events (
           id, actor_kind, actor_admin_user_id, actor_ref, admin_request_id,
           action, entity_kind, entity_ref, config_revision, changed_fields, created_at
         ) values ($1, $2, $3, $4, $5, $6, $7, $8, null, $9, $10)",
    )
    .bind(event.id)
    .bind(event.actor_kind.as_str())
    .bind(event.actor_admin_user_id)
    .bind(event.actor_ref)
    .bind(event.admin_request_id)
    .bind(event.action)
    .bind(event.entity_kind)
    .bind(event.entity_ref)
    .bind(event.changed_fields)
    .bind(event.created_at)
    .execute(&mut **transaction)
    .await
    .map_err(|_| postgres_unavailable("append identity audit event"))?;
    Ok(())
}

async fn ensure_admin_audit_identity_in_transaction(
    tx: &mut Transaction<'_, Postgres>,
    id: &str,
) -> StoreResult<()> {
    // 不生成新身份、不改变角色或已有密码；users 始终是登录凭据的唯一来源。
    sqlx::query(
        "insert into admin_users (id,password_hash,created_at,updated_at)
        select id,password_hash,created_at,updated_at from users where id=$1 and role='admin'
        on conflict(id) do nothing",
    )
    .bind(id)
    .execute(&mut **tx)
    .await
    .map_err(|_| postgres_unavailable("ensure admin audit identity"))?;
    Ok(())
}

/// 自助操作以真实User主体留痕，审计失败与凭据变更在同一事务内回滚。
fn self_service_audit(id: &str, action: &str, field: &str) -> AdminAuditEvent {
    AdminAuditEvent {
        id: format!("audit_{}", uuid::Uuid::now_v7().simple()),
        actor_kind: super::AdminAuditActorKind::UserSession,
        actor_admin_user_id: None,
        actor_ref: format!("user:{id}"),
        admin_request_id: None,
        action: action.to_owned(),
        entity_kind: "user".to_owned(),
        entity_ref: id.to_owned(),
        config_revision: None,
        changed_fields: vec![field.to_owned()],
        created_at: Utc::now(),
    }
}
