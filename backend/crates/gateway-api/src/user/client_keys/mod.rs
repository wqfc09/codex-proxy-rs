//! 普通用户自助 Client API Key HTTP 边界；所有操作都由 Session owner 约束。

use axum::{
    Router,
    extract::State,
    http::{HeaderValue, StatusCode, header::CACHE_CONTROL},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use gateway_admin::model::client_keys::{DeleteClientKey, SetClientKeyEnabled};
use gateway_core::policy::ClientApiKeyId;

use crate::admin::{
    AdminEnvelope, AdminError, AdminJson, AdminQuery, AdminResponse, wire::map_admin_service_error,
};

mod wire;
pub use wire::*;

use super::{UserAuth, UserSessionState};

pub(super) fn router<S>() -> Router<S>
where
    S: UserSessionState + Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/api/user/client-keys", get(list::<S>))
        .route("/api/user/client-keys/create", post(create::<S>))
        .route("/api/user/client-keys/reveal", get(reveal::<S>))
        .route("/api/user/client-keys/update", post(update::<S>))
        .route("/api/user/client-keys/disable", post(disable::<S>))
        .route("/api/user/client-keys/enable", post(enable::<S>))
        .route("/api/user/client-keys/delete", post(delete::<S>))
        .route(
            "/api/user/client-keys/reset-budget",
            post(reset_budget::<S>),
        )
}

async fn reset_budget<S>(
    auth: UserAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<ResetClientKeyBudgetRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: UserSessionState + Send + Sync,
{
    let id = state
        .user_services()
        .client_keys()
        .reset_budget_for_user(
            &auth.user.id,
            request.into_command().map_err(map_wire_error)?,
        )
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(MutatedClientKeyData::new(id.to_string())),
    ))
}

async fn list<S>(
    auth: UserAuth,
    State(state): State<S>,
    AdminQuery(query): AdminQuery<ListClientKeysQuery>,
) -> Result<impl IntoResponse, AdminError>
where
    S: UserSessionState + Send + Sync,
{
    let page = state
        .user_services()
        .client_keys()
        .list_for_user(&auth.user.id, query.into_command().map_err(map_wire_error)?)
        .await
        .map_err(map_admin_service_error)?;
    let data = ClientKeyListData::try_from(page).map_err(|_| AdminError::internal())?;
    Ok(AdminResponse::new(StatusCode::OK, AdminEnvelope::ok(data)))
}

async fn create<S>(
    auth: UserAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<CreateClientKeyRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: UserSessionState + Send + Sync,
{
    let command = request.into_command().map_err(map_wire_error)?;
    let created = state
        .user_services()
        .client_keys()
        .create_for_user(&auth.user.id, command)
        .await
        .map_err(map_admin_service_error)?;
    let mut response = AdminResponse::new(
        StatusCode::CREATED,
        AdminEnvelope::ok(CreatedClientKeyData::from(created)),
    )
    .into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

async fn reveal<S>(
    auth: UserAuth,
    State(state): State<S>,
    AdminQuery(query): AdminQuery<ClientKeyMutationRequest>,
) -> Result<Response, AdminError>
where
    S: UserSessionState + Send + Sync,
{
    let id = client_key_id(query.id)?;
    let secret = state
        .user_services()
        .client_keys()
        .reveal_for_user(&auth.user.id, &id)
        .await
        .map_err(map_admin_service_error)?;
    let mut response = AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(RevealedClientKeyData::from(secret)),
    )
    .into_response();
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

async fn update<S>(
    auth: UserAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<UpdateClientKeyRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: UserSessionState + Send + Sync,
{
    let command = request.into_command().map_err(map_wire_error)?;
    mutation_response(
        state
            .user_services()
            .client_keys()
            .update_for_user(&auth.user.id, command)
            .await,
    )
}

async fn disable<S>(
    auth: UserAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<ClientKeyMutationRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: UserSessionState + Send + Sync,
{
    set_enabled(state, auth, request, false).await
}

async fn enable<S>(
    auth: UserAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<ClientKeyMutationRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: UserSessionState + Send + Sync,
{
    set_enabled(state, auth, request, true).await
}

async fn set_enabled<S>(
    state: S,
    auth: UserAuth,
    request: ClientKeyMutationRequest,
    enabled: bool,
) -> Result<AdminResponse<AdminEnvelope<MutatedClientKeyData>>, AdminError>
where
    S: UserSessionState + Send + Sync,
{
    let id = client_key_id(request.id)?;
    mutation_response(
        state
            .user_services()
            .client_keys()
            .set_enabled_for_user(&auth.user.id, SetClientKeyEnabled { id, enabled })
            .await,
    )
}

async fn delete<S>(
    auth: UserAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<ClientKeyMutationRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: UserSessionState + Send + Sync,
{
    let id = client_key_id(request.id)?;
    mutation_response(
        state
            .user_services()
            .client_keys()
            .delete_for_user(&auth.user.id, DeleteClientKey { id })
            .await,
    )
}

fn mutation_response(
    result: Result<
        gateway_admin::model::client_keys::ClientKeyMutation,
        gateway_admin::model::AdminError,
    >,
) -> Result<AdminResponse<AdminEnvelope<MutatedClientKeyData>>, AdminError> {
    let mutation = result.map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(MutatedClientKeyData::from(mutation)),
    ))
}

fn client_key_id(value: String) -> Result<ClientApiKeyId, AdminError> {
    ClientApiKeyId::new(value).map_err(|_| AdminError::not_found("Client API Key 不存在"))
}

fn map_wire_error(_: crate::admin::WireValidationError) -> AdminError {
    AdminError::bad_request("Client API Key 查询不合法")
}
