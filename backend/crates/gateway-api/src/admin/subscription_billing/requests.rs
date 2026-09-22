use chrono::{DateTime, Duration, Utc};
use gateway_admin::model::{subscription_billing::*, users::UserRole};
use gateway_core::{metering::Decimal, policy::UserBudgetLimits, routing::AccountGroupId};
use serde::Deserialize;

use crate::admin::AdminError;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PlanRequest {
    name: String,
    description: Option<String>,
    daily_limit_usd: Option<String>,
    weekly_limit_usd: Option<String>,
    monthly_limit_usd: Option<String>,
}

impl PlanRequest {
    pub(super) fn into_command(self) -> Result<CreateSubscriptionPlan, AdminError> {
        Ok(CreateSubscriptionPlan {
            name: self.name,
            description: self.description,
            budget_limits: UserBudgetLimits {
                daily_usd: nullable_decimal(self.daily_limit_usd)?,
                weekly_usd: nullable_decimal(self.weekly_limit_usd)?,
                monthly_usd: nullable_decimal(self.monthly_limit_usd)?,
            },
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct UpdatePlanRequest {
    pub(super) id: String,
    name: String,
    description: Option<String>,
    daily_limit_usd: Option<String>,
    weekly_limit_usd: Option<String>,
    monthly_limit_usd: Option<String>,
}

impl UpdatePlanRequest {
    pub(super) fn into_command(self) -> Result<(String, CreateSubscriptionPlan), AdminError> {
        let plan = PlanRequest {
            name: self.name,
            description: self.description,
            daily_limit_usd: self.daily_limit_usd,
            weekly_limit_usd: self.weekly_limit_usd,
            monthly_limit_usd: self.monthly_limit_usd,
        }
        .into_command()?;
        Ok((self.id, plan))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PlanIdRequest {
    pub(super) id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct GrantRequest {
    user_id: String,
    plan_id: String,
    starts_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
    duration_days: Option<u32>,
    multiplier: Option<String>,
}

impl GrantRequest {
    pub(super) fn into_command(self) -> Result<CreateUserSubscription, AdminError> {
        validate_id(&self.user_id)?;
        validate_id(&self.plan_id)?;
        let starts_at = self.starts_at.unwrap_or_else(Utc::now);
        let expires_at = match (self.expires_at, self.duration_days) {
            (Some(end), None) => end,
            (None, Some(days)) => starts_at
                .checked_add_signed(duration(days)?)
                .ok_or_else(|| AdminError::bad_request("订阅日期超出支持范围"))?,
            _ => {
                return Err(AdminError::bad_request(
                    "必须且只能提供 expiresAt 或 durationDays",
                ));
            }
        };
        Ok(CreateUserSubscription {
            user_id: self.user_id,
            plan_id: self.plan_id,
            starts_at,
            expires_at,
            downstream_rate_multiplier: decimal(self.multiplier.as_deref().unwrap_or("1"))?,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PatchSubscriptionRequest {
    user_id: String,
    starts_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
    multiplier: Option<String>,
}

impl PatchSubscriptionRequest {
    pub(super) fn into_command(self) -> Result<UpdateUserSubscription, AdminError> {
        validate_id(&self.user_id)?;
        Ok(UpdateUserSubscription {
            user_id: self.user_id,
            starts_at: self.starts_at,
            expires_at: self.expires_at,
            downstream_rate_multiplier: nullable_decimal(self.multiplier)?,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct RenewRequest {
    user_id: String,
    extend_by_days: Option<u32>,
    expires_at: Option<DateTime<Utc>>,
}

impl RenewRequest {
    pub(super) fn into_command(self) -> Result<RenewUserSubscription, AdminError> {
        validate_id(&self.user_id)?;
        let renewal = match (self.extend_by_days, self.expires_at) {
            (Some(days), None) => {
                duration(days)?;
                SubscriptionRenewal::ExtendByDays(days)
            }
            (None, Some(end)) => SubscriptionRenewal::ExpiresAt(end),
            _ => {
                return Err(AdminError::bad_request(
                    "必须且只能提供 extendByDays 或 expiresAt",
                ));
            }
        };
        Ok(RenewUserSubscription {
            user_id: self.user_id,
            renewal,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct GroupsRequest {
    pub(super) user_id: String,
    pub(super) group_ids: Vec<String>,
}

impl GroupsRequest {
    pub(super) fn into_ids(self) -> Result<(String, Vec<AccountGroupId>), AdminError> {
        validate_id(&self.user_id)?;
        let user_id = self.user_id;
        self.group_ids
            .into_iter()
            .map(|id| {
                validate_id(&id)?;
                AccountGroupId::new(id).map_err(|_| AdminError::bad_request("groupId 不合法"))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|group_ids| (user_id, group_ids))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct TopupRequest {
    user_id: String,
    window: String,
    amount: String,
    idempotency_key: String,
    reason: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ResetBudgetRequest {
    pub(super) user_id: String,
    pub(super) daily: bool,
    pub(super) weekly: bool,
    pub(super) monthly: bool,
}

impl ResetBudgetRequest {
    pub(super) fn into_command(self) -> Result<ResetUserBudget, AdminError> {
        validate_id(&self.user_id)?;
        if !self.daily && !self.weekly && !self.monthly {
            return Err(AdminError::bad_request("至少选择一个额度窗口"));
        }
        Ok(ResetUserBudget {
            user_id: self.user_id,
            daily: self.daily,
            weekly: self.weekly,
            monthly: self.monthly,
        })
    }
}

impl TopupRequest {
    pub(super) fn into_command(self) -> Result<CreateUserBudgetCredit, AdminError> {
        validate_id(&self.user_id)?;
        Ok(CreateUserBudgetCredit {
            user_id: self.user_id,
            window: parse_window(&self.window)?,
            amount: decimal(&self.amount)?,
            idempotency_key: self.idempotency_key,
            reason: self.reason,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct UserIdQuery {
    pub(super) user_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct UserIdRequest {
    pub(super) user_id: String,
}

impl UserIdRequest {
    pub(super) fn into_user_id(self) -> Result<String, AdminError> {
        validate_id(&self.user_id)?;
        Ok(self.user_id)
    }
}

impl UserIdQuery {
    pub(super) fn into_user_id(self) -> Result<String, AdminError> {
        validate_id(&self.user_id)?;
        Ok(self.user_id)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct UserPageQuery {
    pub(super) user_id: String,
    page: Option<u32>,
    page_size: Option<u32>,
}

impl UserPageQuery {
    pub(super) fn into_command(self) -> Result<(String, ControlPageQuery), AdminError> {
        validate_id(&self.user_id)?;
        Ok((self.user_id, pagination(self.page, self.page_size)?))
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct UserTopupQuery {
    pub(super) user_id: String,
    page: Option<u32>,
    page_size: Option<u32>,
    window: Option<String>,
}

impl UserTopupQuery {
    pub(super) fn into_command(
        self,
    ) -> Result<(String, ControlPageQuery, Option<BudgetWindowKind>), AdminError> {
        validate_id(&self.user_id)?;
        Ok((
            self.user_id,
            pagination(self.page, self.page_size)?,
            self.window.as_deref().map(parse_window).transpose()?,
        ))
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ManagementQuery {
    page: Option<u32>,
    page_size: Option<u32>,
    query: Option<String>,
    role: Option<String>,
    status: Option<String>,
    plan_id: Option<String>,
    group_id: Option<String>,
}

impl ManagementQuery {
    pub(super) fn into_command(self) -> Result<UserManagementQuery, AdminError> {
        let role = self
            .role
            .map(|role| {
                UserRole::parse(&role).ok_or_else(|| AdminError::bad_request("role 不合法"))
            })
            .transpose()?;
        let enabled = self
            .status
            .map(|status| match status.as_str() {
                "enabled" => Ok(true),
                "disabled" => Ok(false),
                _ => Err(AdminError::bad_request("status 不合法")),
            })
            .transpose()?;
        if let Some(id) = &self.plan_id {
            validate_id(id)?;
        }
        let group_id = self
            .group_id
            .map(|id| {
                validate_id(&id)?;
                AccountGroupId::new(id).map_err(|_| AdminError::bad_request("groupId 不合法"))
            })
            .transpose()?;
        if self
            .query
            .as_ref()
            .is_some_and(|q| q.len() > 256 || q.chars().any(char::is_control))
        {
            return Err(AdminError::bad_request("query 不合法"));
        }
        Ok(UserManagementQuery {
            pagination: pagination(self.page, self.page_size)?,
            query: self.query.filter(|q| !q.is_empty()),
            role,
            enabled,
            plan_id: self.plan_id,
            group_id,
        })
    }
}

fn pagination(page: Option<u32>, page_size: Option<u32>) -> Result<ControlPageQuery, AdminError> {
    let result = ControlPageQuery {
        page: page.unwrap_or(1),
        page_size: page_size.unwrap_or(20),
    };
    if result.page == 0 || !(1..=100).contains(&result.page_size) {
        return Err(AdminError::bad_request("分页参数不合法"));
    }
    Ok(result)
}

fn duration(days: u32) -> Result<Duration, AdminError> {
    if days == 0 || i64::from(days) * 86400 > MAX_SUBSCRIPTION_DURATION_SECONDS {
        return Err(AdminError::bad_request("期限天数不合法"));
    }
    Ok(Duration::days(i64::from(days)))
}

fn parse_window(value: &str) -> Result<BudgetWindowKind, AdminError> {
    BudgetWindowKind::parse(value).ok_or_else(|| AdminError::bad_request("window 不合法"))
}

fn decimal(value: &str) -> Result<Decimal, AdminError> {
    value
        .parse()
        .map_err(|_| AdminError::bad_request("金额或倍率必须是有效非负十进制字符串"))
}

fn nullable_decimal(value: Option<String>) -> Result<Option<Decimal>, AdminError> {
    value.as_deref().map(decimal).transpose()
}

pub(super) fn validate_id(value: &str) -> Result<(), AdminError> {
    if value.is_empty()
        || value.len() > 128
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(AdminError::bad_request("资源 ID 不合法"));
    }
    Ok(())
}
