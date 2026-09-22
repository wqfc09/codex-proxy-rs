//! 当前账户的个人资料与自助能力 HTTP 边界；Admin/User 共用同一账户会话。

use std::fmt;

use axum::{
    Router,
    extract::{FromRequestParts, State},
    http::{StatusCode, request::Parts},
    response::IntoResponse,
    routing::{get, post},
};
use gateway_admin::{
    AdminServices,
    model::{
        AdminErrorKind,
        users::{UpdateOwnPassword, UpdateOwnUsername, UserRecord},
    },
};
use serde::{Deserialize, Serialize};

use crate::{
    admin::{AdminEnvelope, AdminError, AdminJson, AdminResponse, wire::map_admin_service_error},
    session_cookie,
};

mod billing;
pub mod client_keys;
mod usage;

pub trait UserSessionState {
    fn user_services(&self) -> &AdminServices;
}

pub(super) struct UserAuth {
    pub(super) user: UserRecord,
    session_id: String,
}

impl<S> FromRequestParts<S> for UserAuth
where
    S: UserSessionState + Send + Sync,
{
    type Rejection = AdminError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let session_id =
            session_cookie::value(&parts.headers).ok_or_else(AdminError::session_required)?;
        let user = state
            .user_services()
            .auth()
            .resolve_user(Some(&session_id))
            .await
            .map_err(map_admin_service_error)?
            .ok_or_else(AdminError::session_required)?;
        Ok(Self { user, session_id })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct UserView {
    id: String,
    username: String,
    role: String,
    enabled: bool,
    max_concurrency: Option<u64>,
    requests_per_minute: Option<u64>,
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
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdateUsernameRequest {
    current_password: String,
    username: String,
}

impl fmt::Debug for UpdateUsernameRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UpdateUsernameRequest")
            .field("current_password", &"[REDACTED]")
            .field("username", &self.username)
            .finish()
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpdatePasswordRequest {
    current_password: String,
    new_password: String,
}

impl fmt::Debug for UpdatePasswordRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UpdatePasswordRequest")
            .field("current_password", &"[REDACTED]")
            .field("new_password", &"[REDACTED]")
            .finish()
    }
}

pub fn router<S>() -> Router<S>
where
    S: UserSessionState + Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/api/user/me", get(me::<S>))
        .route("/api/user/profile", post(update_username::<S>))
        .route("/api/user/password", post(update_password::<S>))
        .merge(client_keys::router::<S>())
        .merge(billing::router::<S>())
        .merge(usage::router::<S>())
}

async fn me<S>(auth: UserAuth, State(state): State<S>) -> Result<impl IntoResponse, AdminError>
where
    S: UserSessionState + Send + Sync,
{
    let user = state
        .user_services()
        .users()
        .me(&auth.user.id)
        .await
        .map_err(map_user_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(UserView::from(user)),
    ))
}

async fn update_username<S>(
    auth: UserAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<UpdateUsernameRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: UserSessionState + Send + Sync,
{
    let user = state
        .user_services()
        .users()
        .update_own_username(UpdateOwnUsername {
            user_id: auth.user.id,
            current_session_id: auth.session_id,
            current_password: request.current_password,
            username: request.username,
        })
        .await
        .map_err(map_user_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(UserView::from(user)),
    ))
}

async fn update_password<S>(
    auth: UserAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<UpdatePasswordRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: UserSessionState + Send + Sync,
{
    state
        .user_services()
        .users()
        .update_own_password(UpdateOwnPassword {
            user_id: auth.user.id,
            current_session_id: auth.session_id,
            current_password: request.current_password,
            new_password: request.new_password,
        })
        .await
        .map_err(map_user_service_error)?;
    Ok(AdminResponse::new(StatusCode::OK, AdminEnvelope::ok(())))
}

fn map_user_service_error(error: gateway_admin::model::AdminError) -> AdminError {
    if error.kind() == AdminErrorKind::Unauthorized {
        return AdminError::invalid_request(StatusCode::UNAUTHORIZED, "当前认证或密码无效");
    }
    map_admin_service_error(error)
}
