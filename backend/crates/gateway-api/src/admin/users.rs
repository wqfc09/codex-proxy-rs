//! 管理员用户生命周期与登录防护设置 HTTP 边界。

use std::fmt;

use axum::{
    Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use gateway_admin::model::{
    client_keys::{
        ClientKeyListQuery, ClientKeyPageSize, ClientKeyRecord, ClientKeySort, ClientKeySortField,
        ReplaceClientKeyIdentity, SortDirection,
    },
    users::{CreateUser, ResetUserPassword, SessionTtlSettings, UpdateUser, UserRecord, UserRole},
};
use gateway_core::policy::ClientApiKeyId;
use serde::{Deserialize, Deserializer, Serialize};

use super::{
    AdminAuth, AdminEnvelope, AdminError, AdminJson, AdminQuery, AdminResponse, AdminSessionState,
    wire::map_admin_service_error,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct UserView {
    id: String,
    username: String,
    role: String,
    enabled: bool,
    max_concurrency: Option<u64>,
    requests_per_minute: Option<u64>,
    created_at: String,
    updated_at: String,
}

impl From<UserRecord> for UserView {
    fn from(user: UserRecord) -> Self {
        Self {
            id: user.id,
            username: user.username,
            role: user.role.as_str().to_owned(),
            enabled: user.enabled,
            max_concurrency: user.limits.max_concurrency,
            requests_per_minute: user.limits.requests_per_minute,
            created_at: user.created_at.to_rfc3339(),
            updated_at: user.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateUserRequest {
    username: String,
    password: String,
    enabled: bool,
}

impl fmt::Debug for CreateUserRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreateUserRequest")
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .field("enabled", &self.enabled)
            .finish()
    }
}

fn nullable_patch<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateUserRequest {
    user_id: String,
    username: Option<String>,
    role: Option<String>,
    enabled: Option<bool>,
    #[serde(default, deserialize_with = "nullable_patch")]
    max_concurrency: Option<Option<u64>>,
    #[serde(default, deserialize_with = "nullable_patch")]
    requests_per_minute: Option<Option<u64>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResetPasswordRequest {
    user_id: String,
    password: String,
}

impl fmt::Debug for ResetPasswordRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResetPasswordRequest")
            .field("password", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct InvalidatedSessionsView {
    invalidated: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UserIdRequest {
    user_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionTtlSettingsView {
    admin_minutes: u64,
    user_minutes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateSessionTtlRequest {
    admin_minutes: u64,
    user_minutes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct TurnstileSettingsView {
    enabled: bool,
    site_key: Option<String>,
    has_secret: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateTurnstileRequest {
    enabled: bool,
    site_key: Option<String>,
    secret_key: Option<String>,
}

impl fmt::Debug for UpdateTurnstileRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UpdateTurnstileRequest")
            .field("enabled", &self.enabled)
            .field("site_key", &self.site_key)
            .field(
                "secret_key",
                &self.secret_key.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UserKeyIdentityKeyView {
    id: String,
    name: String,
    label: Option<String>,
    prefix: String,
    enabled: bool,
    openai_client_profile_override: Option<serde_json::Map<String, serde_json::Value>>,
    xai_client_profile_override: Option<serde_json::Map<String, serde_json::Value>>,
}

impl From<ClientKeyRecord> for UserKeyIdentityKeyView {
    fn from(record: ClientKeyRecord) -> Self {
        Self {
            id: record.id.to_string(),
            name: record.name,
            label: record.label,
            prefix: record.prefix,
            enabled: record.enabled,
            openai_client_profile_override: record
                .openai_client_profile_override
                .map(gateway_core::account::OpaqueProviderData::into_inner),
            xai_client_profile_override: record
                .xai_client_profile_override
                .map(gateway_core::account::OpaqueProviderData::into_inner),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UserKeyIdentityView {
    openai_client_profile_override: Option<serde_json::Map<String, serde_json::Value>>,
    xai_client_profile_override: Option<serde_json::Map<String, serde_json::Value>>,
    key_count: u64,
    keys: Vec<UserKeyIdentityKeyView>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateUserKeyIdentityRequest {
    user_id: String,
    openai_client_profile_override: Option<serde_json::Map<String, serde_json::Value>>,
    xai_client_profile_override: Option<serde_json::Map<String, serde_json::Value>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateUserClientKeyIdentityRequest {
    user_id: String,
    key_id: String,
    openai_client_profile_override: Option<serde_json::Map<String, serde_json::Value>>,
    xai_client_profile_override: Option<serde_json::Map<String, serde_json::Value>>,
}

async fn key_identity<S>(
    _auth: AdminAuth,
    State(state): State<S>,
    AdminQuery(request): AdminQuery<UserIdRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    validate_user_id(&request.user_id)?;
    let identity = state
        .admin_services()
        .users()
        .key_identity(&request.user_id)
        .await
        .map_err(map_admin_service_error)?;
    let key_page = state
        .admin_services()
        .client_keys()
        .list_for_user(
            &request.user_id,
            ClientKeyListQuery {
                cursor: None,
                page_size: ClientKeyPageSize::new(u16::MAX).map_err(|_| AdminError::internal())?,
                search: None,
                sort: ClientKeySort {
                    field: ClientKeySortField::Name,
                    direction: SortDirection::Asc,
                },
            },
        )
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(UserKeyIdentityView {
            openai_client_profile_override: identity
                .openai
                .map(gateway_core::account::OpaqueProviderData::into_inner),
            xai_client_profile_override: identity
                .xai
                .map(gateway_core::account::OpaqueProviderData::into_inner),
            key_count: key_page.total,
            keys: key_page.items.into_iter().map(Into::into).collect(),
        }),
    ))
}

async fn update_client_key_identity<S>(
    auth: AdminAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<UpdateUserClientKeyIdentityRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    validate_user_id(&request.user_id)?;
    let key_id =
        ClientApiKeyId::new(request.key_id).map_err(|_| AdminError::bad_request("keyId 不合法"))?;
    state
        .admin_services()
        .client_keys()
        .replace_identity_for_user(
            &auth.context().mutation_context(),
            &request.user_id,
            ReplaceClientKeyIdentity {
                id: key_id,
                openai_client_profile_override: request
                    .openai_client_profile_override
                    .map(gateway_core::account::OpaqueProviderData::new),
                xai_client_profile_override: request
                    .xai_client_profile_override
                    .map(gateway_core::account::OpaqueProviderData::new),
            },
        )
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(StatusCode::OK, AdminEnvelope::ok(())))
}

async fn update_key_identity<S>(
    auth: AdminAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<UpdateUserKeyIdentityRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    validate_user_id(&request.user_id)?;
    state
        .admin_services()
        .users()
        .replace_key_identity(
            &auth.context().mutation_context(),
            &request.user_id,
            gateway_admin::model::users::UserKeyIdentity {
                openai: request
                    .openai_client_profile_override
                    .map(gateway_core::account::OpaqueProviderData::new),
                xai: request
                    .xai_client_profile_override
                    .map(gateway_core::account::OpaqueProviderData::new),
            },
        )
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(StatusCode::OK, AdminEnvelope::ok(())))
}

pub fn router<S>() -> Router<S>
where
    S: AdminSessionState + Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/api/admin/users", get(list::<S>))
        .route("/api/admin/users/create", post(create::<S>))
        .route("/api/admin/users/update", post(update::<S>))
        .route("/api/admin/users/key-identity", get(key_identity::<S>))
        .route(
            "/api/admin/users/key-identity/update",
            post(update_key_identity::<S>),
        )
        .route(
            "/api/admin/users/key-identity/key/update",
            post(update_client_key_identity::<S>),
        )
        .route("/api/admin/users/reset-password", post(reset_password::<S>))
        .route(
            "/api/admin/users/revoke-sessions",
            post(revoke_sessions::<S>),
        )
        .route("/api/admin/users/delete", post(delete_user::<S>))
        .route("/api/admin/auth/session", get(session_ttl_settings::<S>))
        .route(
            "/api/admin/auth/session/update",
            post(update_session_ttl_settings::<S>),
        )
        .route("/api/admin/auth/turnstile", get(turnstile_settings::<S>))
        .route(
            "/api/admin/auth/turnstile/update",
            post(update_turnstile_settings::<S>),
        )
}

async fn list<S>(_auth: AdminAuth, State(state): State<S>) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    let users = state
        .admin_services()
        .users()
        .list()
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(users.into_iter().map(UserView::from).collect::<Vec<_>>()),
    ))
}

async fn create<S>(
    auth: AdminAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<CreateUserRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    let user = state
        .admin_services()
        .users()
        .create(
            &auth.context().mutation_context(),
            CreateUser {
                username: request.username,
                password: request.password,
                role: UserRole::User,
                enabled: request.enabled,
            },
        )
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::CREATED,
        AdminEnvelope::ok(UserView::from(user)),
    ))
}

async fn update<S>(
    auth: AdminAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<UpdateUserRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    let UpdateUserRequest {
        user_id,
        username,
        role: role_name,
        enabled,
        max_concurrency,
        requests_per_minute,
    } = request;
    validate_user_id(&user_id)?;
    let role = role_name
        .as_deref()
        .map(|value| {
            UserRole::parse(value).ok_or_else(|| AdminError::bad_request("账户角色不合法"))
        })
        .transpose()?;
    let user = state
        .admin_services()
        .users()
        .update(
            &auth.context().mutation_context(),
            UpdateUser {
                user_id,
                username,
                role,
                enabled,
                max_concurrency,
                requests_per_minute,
            },
        )
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(UserView::from(user)),
    ))
}

async fn reset_password<S>(
    auth: AdminAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<ResetPasswordRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    validate_user_id(&request.user_id)?;
    state
        .admin_services()
        .users()
        .reset_password(
            &auth.context().mutation_context(),
            ResetUserPassword {
                user_id: request.user_id,
                password: request.password,
            },
        )
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(StatusCode::OK, AdminEnvelope::ok(())))
}

async fn delete_user<S>(
    auth: AdminAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<UserIdRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    validate_user_id(&request.user_id)?;
    state
        .admin_services()
        .users()
        .delete(&auth.context().mutation_context(), &request.user_id)
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(StatusCode::OK, AdminEnvelope::ok(())))
}

async fn revoke_sessions<S>(
    auth: AdminAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<UserIdRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    validate_user_id(&request.user_id)?;
    state
        .admin_services()
        .users()
        .revoke_sessions(&auth.context().mutation_context(), &request.user_id)
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(InvalidatedSessionsView { invalidated: true }),
    ))
}

fn validate_user_id(user_id: &str) -> Result<(), AdminError> {
    if user_id.is_empty()
        || user_id.len() > 128
        || user_id.trim() != user_id
        || user_id.chars().any(char::is_control)
    {
        return Err(AdminError::bad_request("用户 ID 不合法"));
    }
    Ok(())
}

async fn session_ttl_settings<S>(
    _auth: AdminAuth,
    State(state): State<S>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    let settings = state
        .admin_services()
        .users()
        .session_ttl_settings()
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(SessionTtlSettingsView {
            admin_minutes: settings.admin_minutes,
            user_minutes: settings.user_minutes,
        }),
    ))
}

async fn update_session_ttl_settings<S>(
    auth: AdminAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<UpdateSessionTtlRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    let settings = state
        .admin_services()
        .users()
        .replace_session_ttl_settings(
            &auth.context().mutation_context(),
            SessionTtlSettings::new(request.admin_minutes, request.user_minutes),
        )
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(SessionTtlSettingsView {
            admin_minutes: settings.admin_minutes,
            user_minutes: settings.user_minutes,
        }),
    ))
}

async fn turnstile_settings<S>(
    _auth: AdminAuth,
    State(state): State<S>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    let settings = state
        .admin_services()
        .users()
        .turnstile_settings()
        .await
        .map_err(map_admin_service_error)?;
    let has_secret = settings.has_secret();
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(TurnstileSettingsView {
            enabled: settings.enabled,
            site_key: settings.site_key,
            has_secret,
        }),
    ))
}

async fn update_turnstile_settings<S>(
    auth: AdminAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<UpdateTurnstileRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    let settings = state
        .admin_services()
        .users()
        .replace_turnstile_settings(
            &auth.context().mutation_context(),
            request.enabled,
            request.site_key,
            request.secret_key,
        )
        .await
        .map_err(map_admin_service_error)?;
    let has_secret = settings.has_secret();
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(TurnstileSettingsView {
            enabled: settings.enabled,
            site_key: settings.site_key,
            has_secret,
        }),
    ))
}
