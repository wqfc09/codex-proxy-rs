use chrono::{DateTime, Utc};
use gateway_core::{metering::Decimal, policy::UserBudgetLimits, routing::AccountGroupId};

use super::{Revision, users::UserRole};

/// 单次订阅的最长跨度为一百个 365 天年度。
pub const MAX_SUBSCRIPTION_DURATION_SECONDS: i64 = 3_153_600_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionPlanRecord {
    pub id: String,
    pub is_base: bool,
    pub name: String,
    pub description: Option<String>,
    pub enabled: bool,
    pub budget_limits: UserBudgetLimits,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateSubscriptionPlan {
    pub name: String,
    pub description: Option<String>,
    pub budget_limits: UserBudgetLimits,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSubscriptionPlan {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub budget_limits: UserBudgetLimits,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateSubscriptionPlan {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub budget_limits: UserBudgetLimits,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetSubscriptionPlanEnabled {
    pub id: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionPlanMutation {
    pub config_revision: Revision,
    pub record: SubscriptionPlanRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserSubscriptionRecord {
    pub id: String,
    pub user_id: Option<String>,
    pub plan_id: String,
    pub plan_name: String,
    pub status: String,
    pub starts_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub downstream_rate_multiplier: Decimal,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl UserSubscriptionRecord {
    #[must_use]
    pub fn effective_status(&self, now: DateTime<Utc>) -> &'static str {
        if self.status == "revoked" {
            "revoked"
        } else if self.starts_at > now {
            "pending"
        } else if self.expires_at <= now {
            "expired"
        } else {
            "active"
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateUserSubscription {
    pub user_id: String,
    pub plan_id: String,
    pub starts_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub downstream_rate_multiplier: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateUserBudgetCredit {
    pub user_id: String,
    pub window: BudgetWindowKind,
    pub amount: Decimal,
    pub idempotency_key: String,
    pub reason: String,
}

/// 清理用户当前额度窗口；历史计费事件与套餐事实不受影响。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetUserBudget {
    pub user_id: String,
    pub daily: bool,
    pub weekly: bool,
    pub monthly: bool,
}

/// 时间和倍率已经由用例校验；套餐不再决定期限。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantUserSubscription {
    pub id: String,
    pub user_id: String,
    pub plan_id: String,
    pub starts_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub downstream_rate_multiplier: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateUserSubscription {
    pub user_id: String,
    pub starts_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub downstream_rate_multiplier: Option<Decimal>,
}

/// 相对续期必须在锁住用户后基于原到期时间计算。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscriptionRenewal {
    ExtendByDays(u32),
    ExpiresAt(DateTime<Utc>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenewUserSubscription {
    pub user_id: String,
    pub renewal: SubscriptionRenewal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserSubscriptionMutation {
    pub config_revision: Revision,
    pub record: UserSubscriptionRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserBudgetWindowStatus {
    pub limit: Option<Decimal>,
    pub used: Decimal,
    pub credit: Decimal,
    pub effective_limit: Option<Decimal>,
    pub resets_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserBudgetStatus {
    pub daily: UserBudgetWindowStatus,
    pub weekly: UserBudgetWindowStatus,
    pub monthly: UserBudgetWindowStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserBillingSummary {
    pub subscription: Option<UserSubscriptionRecord>,
    /// 必须是当前有效订阅套餐或真实基础套餐；缺失为配置错误。
    pub plan: SubscriptionPlanRecord,
    pub subscription_history: Vec<UserSubscriptionRecord>,
    pub effective_max_concurrency: Option<u64>,
    pub effective_requests_per_minute: Option<u64>,
    pub budget: UserBudgetStatus,
    pub groups: Vec<UserGroupRecord>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetWindowKind {
    Daily,
    Weekly,
    Monthly,
}

impl BudgetWindowKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::Monthly => "monthly",
        }
    }
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "daily" => Some(Self::Daily),
            "weekly" => Some(Self::Weekly),
            "monthly" => Some(Self::Monthly),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserBudgetCreditRecord {
    pub id: String,
    pub user_id: String,
    pub plan_id: String,
    pub subscription_id: Option<String>,
    pub window: BudgetWindowKind,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub amount: Decimal,
    pub reason: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddUserBudgetCredit {
    pub id: String,
    pub user_id: String,
    pub window: BudgetWindowKind,
    pub amount: Decimal,
    pub idempotency_key: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserGroupRecord {
    pub group_id: AccountGroupId,
    pub name: String,
    pub enabled: bool,
    pub assigned_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserGroups {
    pub items: Vec<UserGroupRecord>,
    pub revision: Revision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlPageQuery {
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlPage<T> {
    pub items: Vec<T>,
    pub page: u32,
    pub page_size: u32,
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserManagementQuery {
    pub pagination: ControlPageQuery,
    pub query: Option<String>,
    pub role: Option<UserRole>,
    pub enabled: Option<bool>,
    pub plan_id: Option<String>,
    pub group_id: Option<AccountGroupId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserManagementRecord {
    pub user_id: String,
    pub username: String,
    pub role: UserRole,
    pub enabled: bool,
    pub updated_at: DateTime<Utc>,
    pub billing: UserBillingSummary,
}
