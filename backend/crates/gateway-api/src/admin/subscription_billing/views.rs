use chrono::{DateTime, Utc};
use gateway_admin::model::subscription_billing::*;
use gateway_core::metering::Decimal;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PlanView {
    id: String,
    name: String,
    description: Option<String>,
    enabled: bool,
    is_base: bool,
    daily_limit_usd: Option<String>,
    weekly_limit_usd: Option<String>,
    monthly_limit_usd: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<SubscriptionPlanRecord> for PlanView {
    fn from(plan: SubscriptionPlanRecord) -> Self {
        Self {
            id: plan.id,
            name: plan.name,
            description: plan.description,
            enabled: plan.enabled,
            is_base: plan.is_base,
            daily_limit_usd: plan.budget_limits.daily_usd.map(Decimal::canonical),
            weekly_limit_usd: plan.budget_limits.weekly_usd.map(Decimal::canonical),
            monthly_limit_usd: plan.budget_limits.monthly_usd.map(Decimal::canonical),
            created_at: plan.created_at,
            updated_at: plan.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SubscriptionView {
    #[serde(flatten)]
    safe: SafeSubscriptionView,
    user_id: Option<String>,
}

impl From<UserSubscriptionRecord> for SubscriptionView {
    fn from(record: UserSubscriptionRecord) -> Self {
        Self {
            user_id: record.user_id.clone(),
            safe: record.into(),
        }
    }
}

/// 个人账单字段白名单，新增管理字段不会自动进入个人响应。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SafeSubscriptionView {
    id: String,
    plan_id: String,
    plan_name: String,
    status: String,
    effective_status: String,
    starts_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
    multiplier: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<UserSubscriptionRecord> for SafeSubscriptionView {
    fn from(record: UserSubscriptionRecord) -> Self {
        let effective_status = record.effective_status(Utc::now()).to_owned();
        Self {
            id: record.id,
            plan_id: record.plan_id,
            plan_name: record.plan_name,
            status: record.status,
            effective_status,
            starts_at: record.starts_at,
            expires_at: record.expires_at,
            revoked_at: record.revoked_at,
            multiplier: record.downstream_rate_multiplier.canonical(),
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BudgetWindowView {
    limit: Option<String>,
    used: String,
    credit: String,
    effective_limit: Option<String>,
    resets_at: DateTime<Utc>,
}

impl From<UserBudgetWindowStatus> for BudgetWindowView {
    fn from(value: UserBudgetWindowStatus) -> Self {
        Self {
            limit: value.limit.map(Decimal::canonical),
            used: value.used.canonical(),
            credit: value.credit.canonical(),
            effective_limit: value.effective_limit.map(Decimal::canonical),
            resets_at: value.resets_at,
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct BudgetView {
    daily: BudgetWindowView,
    weekly: BudgetWindowView,
    monthly: BudgetWindowView,
}

impl From<UserBudgetStatus> for BudgetView {
    fn from(value: UserBudgetStatus) -> Self {
        Self {
            daily: value.daily.into(),
            weekly: value.weekly.into(),
            monthly: value.monthly.into(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SafeGroupView {
    group_id: String,
    name: String,
    enabled: bool,
}

impl From<UserGroupRecord> for SafeGroupView {
    fn from(value: UserGroupRecord) -> Self {
        Self {
            group_id: value.group_id.as_str().to_owned(),
            name: value.name,
            enabled: value.enabled,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct GroupView {
    #[serde(flatten)]
    safe: SafeGroupView,
    assigned_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub(super) struct GroupsView {
    items: Vec<GroupView>,
    revision: u64,
}

impl From<UserGroups> for GroupsView {
    fn from(value: UserGroups) -> Self {
        Self {
            items: value
                .items
                .into_iter()
                .map(|item| GroupView {
                    assigned_at: item.assigned_at,
                    safe: item.into(),
                })
                .collect(),
            revision: value.revision.get(),
        }
    }
}

pub(crate) fn effective_source(summary: &UserBillingSummary) -> &'static str {
    if summary.subscription.is_some() {
        "subscription"
    } else {
        "basePlan"
    }
}

pub(crate) fn effective_multiplier(summary: &UserBillingSummary) -> String {
    summary.subscription.as_ref().map_or_else(
        || "1".to_owned(),
        |s| s.downstream_rate_multiplier.canonical(),
    )
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BillingView {
    plan: PlanView,
    subscription: Option<SubscriptionView>,
    effective_source: &'static str,
    history: Vec<SubscriptionView>,
    effective_max_concurrency: Option<u64>,
    effective_requests_per_minute: Option<u64>,
    multiplier: String,
    budget: BudgetView,
    groups: Vec<SafeGroupView>,
}

impl From<UserBillingSummary> for BillingView {
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ManagementView {
    user_id: String,
    username: String,
    email: Option<String>,
    role: &'static str,
    enabled: bool,
    effective_plan: PlanView,
    effective_source: &'static str,
    subscription: Option<SubscriptionView>,
    multiplier: String,
    groups: Vec<SafeGroupView>,
    budget: BudgetView,
    effective_max_concurrency: Option<u64>,
    effective_requests_per_minute: Option<u64>,
    updated_at: DateTime<Utc>,
}

impl From<UserManagementRecord> for ManagementView {
    fn from(value: UserManagementRecord) -> Self {
        let billing = value.billing;
        Self {
            user_id: value.user_id,
            username: value.username,
            email: None,
            role: value.role.as_str(),
            enabled: value.enabled,
            updated_at: value.updated_at,
            effective_source: effective_source(&billing),
            multiplier: effective_multiplier(&billing),
            effective_plan: billing.plan.into(),
            subscription: billing.subscription.map(Into::into),
            groups: billing.groups.into_iter().map(Into::into).collect(),
            budget: billing.budget.into(),
            effective_max_concurrency: billing.effective_max_concurrency,
            effective_requests_per_minute: billing.effective_requests_per_minute,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreditView {
    id: String,
    user_id: String,
    plan_id: String,
    subscription_id: Option<String>,
    window: &'static str,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
    amount: String,
    reason: String,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct BudgetResetView {
    daily: bool,
    weekly: bool,
    monthly: bool,
}

impl From<ResetUserBudget> for BudgetResetView {
    fn from(value: ResetUserBudget) -> Self {
        Self {
            daily: value.daily,
            weekly: value.weekly,
            monthly: value.monthly,
        }
    }
}

impl From<UserBudgetCreditRecord> for CreditView {
    fn from(value: UserBudgetCreditRecord) -> Self {
        Self {
            id: value.id,
            user_id: value.user_id,
            plan_id: value.plan_id,
            subscription_id: value.subscription_id,
            window: value.window.as_str(),
            window_start: value.window_start,
            window_end: value.window_end,
            amount: value.amount.canonical(),
            reason: value.reason,
            created_at: value.created_at,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PageView<T> {
    items: Vec<T>,
    page: u32,
    page_size: u32,
    total: u64,
}

impl<T> PageView<T> {
    pub(super) fn from_page<R: Into<T>>(page: ControlPage<R>) -> Self {
        Self {
            items: page.items.into_iter().map(Into::into).collect(),
            page: page.page,
            page_size: page.page_size,
            total: page.total,
        }
    }
}
