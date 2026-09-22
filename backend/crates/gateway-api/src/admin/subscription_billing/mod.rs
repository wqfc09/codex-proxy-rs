//! 套餐、用户策略与补额的管理员 HTTP 边界。

mod requests;
pub(crate) mod views;

use axum::{
    Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use gateway_admin::model::subscription_billing::{
    SetSubscriptionPlanEnabled, UpdateSubscriptionPlan,
};
use serde::Serialize;

use super::{
    AdminAuth, AdminEnvelope, AdminError, AdminJson, AdminQuery, AdminResponse, AdminSessionState,
    wire::map_admin_service_error,
};
use requests::*;
use views::*;

pub fn router<S>() -> Router<S>
where
    S: AdminSessionState + Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/api/admin/subscription-plans", get(list_plans::<S>))
        .route(
            "/api/admin/subscription-plans/create",
            post(create_plan::<S>),
        )
        .route(
            "/api/admin/subscription-plans/update",
            post(update_plan::<S>),
        )
        .route(
            "/api/admin/subscription-plans/enable",
            post(enable_plan::<S>),
        )
        .route(
            "/api/admin/subscription-plans/disable",
            post(disable_plan::<S>),
        )
        .route("/api/admin/users/management", get(management::<S>))
        .route("/api/admin/users/billing", get(user_billing::<S>))
        .route(
            "/api/admin/users/subscription/grant",
            post(grant_subscription::<S>),
        )
        .route(
            "/api/admin/users/subscription/update",
            post(update_subscription::<S>),
        )
        .route(
            "/api/admin/users/subscription/renew",
            post(renew_subscription::<S>),
        )
        .route(
            "/api/admin/users/subscription/revoke",
            post(revoke_subscription::<S>),
        )
        .route(
            "/api/admin/users/subscription-history",
            get(subscription_history::<S>),
        )
        .route("/api/admin/users/groups", get(user_groups::<S>))
        .route("/api/admin/users/groups/update", post(replace_groups::<S>))
        .route("/api/admin/users/billing/topups", get(topups::<S>))
        .route(
            "/api/admin/users/billing/topups/create",
            post(add_topup::<S>),
        )
        .route("/api/admin/users/billing/reset", post(reset_budget::<S>))
}

fn ok<T: Serialize>(data: T) -> AdminResponse<AdminEnvelope<T>> {
    AdminResponse::new(StatusCode::OK, AdminEnvelope::ok(data))
}

async fn list_plans<S>(
    _auth: AdminAuth,
    State(state): State<S>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    let records = state
        .admin_services()
        .subscription_billing()
        .plans()
        .await
        .map_err(map_admin_service_error)?;
    Ok(ok(records
        .into_iter()
        .map(PlanView::from)
        .collect::<Vec<_>>()))
}

async fn create_plan<S>(
    auth: AdminAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<PlanRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    let mutation = state
        .admin_services()
        .subscription_billing()
        .create_plan(&auth.context().mutation_context(), request.into_command()?)
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::CREATED,
        AdminEnvelope::ok(PlanView::from(mutation.record)),
    ))
}

async fn update_plan<S>(
    auth: AdminAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<UpdatePlanRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    let (id, plan) = request.into_command()?;
    validate_id(&id)?;
    let mutation = state
        .admin_services()
        .subscription_billing()
        .update_plan(
            &auth.context().mutation_context(),
            UpdateSubscriptionPlan {
                id,
                name: plan.name,
                description: plan.description,
                budget_limits: plan.budget_limits,
            },
        )
        .await
        .map_err(map_admin_service_error)?;
    Ok(ok(PlanView::from(mutation.record)))
}

async fn enable_plan<S>(
    auth: AdminAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<PlanIdRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    set_plan_enabled(state, auth, request.id, true).await
}

async fn disable_plan<S>(
    auth: AdminAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<PlanIdRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    set_plan_enabled(state, auth, request.id, false).await
}

async fn set_plan_enabled<S>(
    state: S,
    auth: AdminAuth,
    id: String,
    enabled: bool,
) -> Result<AdminResponse<AdminEnvelope<PlanView>>, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    validate_id(&id)?;
    let mutation = state
        .admin_services()
        .subscription_billing()
        .set_plan_enabled(
            &auth.context().mutation_context(),
            SetSubscriptionPlanEnabled { id, enabled },
        )
        .await
        .map_err(map_admin_service_error)?;
    Ok(ok(PlanView::from(mutation.record)))
}

async fn management<S>(
    _auth: AdminAuth,
    State(state): State<S>,
    AdminQuery(query): AdminQuery<ManagementQuery>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    let page = state
        .admin_services()
        .subscription_billing()
        .management(query.into_command()?)
        .await
        .map_err(map_admin_service_error)?;
    Ok(ok(PageView::<ManagementView>::from_page(page)))
}

async fn user_billing<S>(
    _auth: AdminAuth,
    State(state): State<S>,
    AdminQuery(query): AdminQuery<UserIdQuery>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    let user_id = query.into_user_id()?;
    let summary = state
        .admin_services()
        .subscription_billing()
        .user_summary(&user_id)
        .await
        .map_err(map_admin_service_error)?;
    Ok(ok(BillingView::from(summary)))
}

async fn grant_subscription<S>(
    auth: AdminAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<GrantRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    let mutation = state
        .admin_services()
        .subscription_billing()
        .grant_subscription(&auth.context().mutation_context(), request.into_command()?)
        .await
        .map_err(map_admin_service_error)?;
    Ok(ok(SubscriptionView::from(mutation.record)))
}

async fn update_subscription<S>(
    auth: AdminAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<PatchSubscriptionRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    let mutation = state
        .admin_services()
        .subscription_billing()
        .update_subscription(&auth.context().mutation_context(), request.into_command()?)
        .await
        .map_err(map_admin_service_error)?;
    Ok(ok(SubscriptionView::from(mutation.record)))
}

async fn renew_subscription<S>(
    auth: AdminAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<RenewRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    let mutation = state
        .admin_services()
        .subscription_billing()
        .renew_subscription(&auth.context().mutation_context(), request.into_command()?)
        .await
        .map_err(map_admin_service_error)?;
    Ok(ok(SubscriptionView::from(mutation.record)))
}

async fn revoke_subscription<S>(
    auth: AdminAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<UserIdRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    let user_id = request.into_user_id()?;
    let mutation = state
        .admin_services()
        .subscription_billing()
        .revoke_subscription(&auth.context().mutation_context(), &user_id)
        .await
        .map_err(map_admin_service_error)?;
    Ok(ok(mutation.map(|m| SubscriptionView::from(m.record))))
}

async fn subscription_history<S>(
    _auth: AdminAuth,
    State(state): State<S>,
    AdminQuery(query): AdminQuery<UserPageQuery>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    let (user_id, page_query) = query.into_command()?;
    let page = state
        .admin_services()
        .subscription_billing()
        .subscription_history(&user_id, page_query)
        .await
        .map_err(map_admin_service_error)?;
    Ok(ok(PageView::<SubscriptionView>::from_page(page)))
}

async fn user_groups<S>(
    _auth: AdminAuth,
    State(state): State<S>,
    AdminQuery(query): AdminQuery<UserIdQuery>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    let user_id = query.into_user_id()?;
    let groups = state
        .admin_services()
        .subscription_billing()
        .user_groups(&user_id)
        .await
        .map_err(map_admin_service_error)?;
    Ok(ok(GroupsView::from(groups)))
}

async fn replace_groups<S>(
    auth: AdminAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<GroupsRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    let (user_id, group_ids) = request.into_ids()?;
    let groups = state
        .admin_services()
        .subscription_billing()
        .replace_user_groups(&auth.context().mutation_context(), &user_id, group_ids)
        .await
        .map_err(map_admin_service_error)?;
    Ok(ok(GroupsView::from(groups)))
}

async fn topups<S>(
    _auth: AdminAuth,
    State(state): State<S>,
    AdminQuery(query): AdminQuery<UserTopupQuery>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    let (user_id, pagination, window) = query.into_command()?;
    let page = state
        .admin_services()
        .subscription_billing()
        .budget_credits(&user_id, window, pagination)
        .await
        .map_err(map_admin_service_error)?;
    Ok(ok(PageView::<CreditView>::from_page(page)))
}

async fn add_topup<S>(
    auth: AdminAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<TopupRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    let record = state
        .admin_services()
        .subscription_billing()
        .add_budget_credit(&auth.context().mutation_context(), request.into_command()?)
        .await
        .map_err(map_admin_service_error)?;
    Ok(ok(CreditView::from(record)))
}

async fn reset_budget<S>(
    auth: AdminAuth,
    State(state): State<S>,
    AdminJson(request): AdminJson<ResetBudgetRequest>,
) -> Result<impl IntoResponse, AdminError>
where
    S: AdminSessionState + Send + Sync,
{
    let reset = state
        .admin_services()
        .subscription_billing()
        .reset_budget(&auth.context().mutation_context(), request.into_command()?)
        .await
        .map_err(map_admin_service_error)?;
    Ok(ok(BudgetResetView::from(reset)))
}
