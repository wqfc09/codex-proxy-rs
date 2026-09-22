//! 普通用户仅通过独立安全投影读取自己的套餐、限额和分组。

use axum::{
    Router,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use gateway_admin::model::subscription_billing::{ControlPageQuery, UserBillingSummary};
use serde::{Deserialize, Serialize};

use super::{UserAuth, UserSessionState};
use crate::admin::{
    AdminEnvelope, AdminError, AdminResponse,
    subscription_billing::views::{
        BudgetView, PlanView, SafeGroupView, SafeSubscriptionView, effective_multiplier,
        effective_source,
    },
    wire::map_admin_service_error,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PersonalBillingView {
    plan: PlanView,
    effective_source: &'static str,
    subscription: Option<SafeSubscriptionView>,
    history: Vec<SafeSubscriptionView>,
    effective_max_concurrency: Option<u64>,
    effective_requests_per_minute: Option<u64>,
    multiplier: String,
    budget: BudgetView,
    groups: Vec<SafeGroupView>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SubscriptionPageQuery {
    page: Option<u32>,
    page_size: Option<u32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PersonalSubscriptionPageView {
    items: Vec<SafeSubscriptionView>,
    page: u32,
    page_size: u32,
    total: u64,
}

impl From<UserBillingSummary> for PersonalBillingView {
    fn from(value: UserBillingSummary) -> Self {
        Self {
            effective_source: effective_source(&value),
            multiplier: effective_multiplier(&value),
            plan: value.plan.into(),
            subscription: value.subscription.map(Into::into),
            history: value
                .subscription_history
                .into_iter()
                .map(Into::into)
                .collect(),
            effective_max_concurrency: value.effective_max_concurrency,
            effective_requests_per_minute: value.effective_requests_per_minute,
            budget: value.budget.into(),
            groups: value.groups.into_iter().map(Into::into).collect(),
        }
    }
}

pub(super) fn router<S>() -> Router<S>
where
    S: UserSessionState + Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/api/user/billing", get(summary::<S>))
        .route(
            "/api/user/billing/subscriptions",
            get(subscription_history::<S>),
        )
        .route("/api/user/groups", get(groups::<S>))
}

async fn summary<S>(auth: UserAuth, State(state): State<S>) -> Result<impl IntoResponse, AdminError>
where
    S: UserSessionState + Send + Sync,
{
    let summary = state
        .user_services()
        .subscription_billing()
        .user_summary(&auth.user.id)
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(PersonalBillingView::from(summary)),
    ))
}

async fn subscription_history<S>(
    auth: UserAuth,
    State(state): State<S>,
    Query(query): Query<SubscriptionPageQuery>,
) -> Result<impl IntoResponse, AdminError>
where
    S: UserSessionState + Send + Sync,
{
    let page = state
        .user_services()
        .subscription_billing()
        .subscription_history(
            &auth.user.id,
            ControlPageQuery {
                page: query.page.unwrap_or(1),
                page_size: query.page_size.unwrap_or(5),
            },
        )
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(PersonalSubscriptionPageView {
            items: page.items.into_iter().map(Into::into).collect(),
            page: page.page,
            page_size: page.page_size,
            total: page.total,
        }),
    ))
}

async fn groups<S>(auth: UserAuth, State(state): State<S>) -> Result<impl IntoResponse, AdminError>
where
    S: UserSessionState + Send + Sync,
{
    let groups = state
        .user_services()
        .subscription_billing()
        .user_groups(&auth.user.id)
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(
            groups
                .items
                .into_iter()
                .map(SafeGroupView::from)
                .collect::<Vec<_>>(),
        ),
    ))
}
