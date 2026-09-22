use std::sync::Arc;

use async_trait::async_trait;
use gateway_core::runtime::SnapshotControl;
use uuid::Uuid;

use crate::{
    model::{AdminError, MutationContext, subscription_billing::*},
    ports::store::SubscriptionBillingStore,
};

use super::{map_store_error, publish_committed};

#[async_trait]
pub trait SubscriptionBillingService: Send + Sync {
    async fn plans(&self) -> Result<Vec<SubscriptionPlanRecord>, AdminError>;
    async fn create_plan(
        &self,
        context: &MutationContext,
        command: CreateSubscriptionPlan,
    ) -> Result<SubscriptionPlanMutation, AdminError>;

    async fn update_plan(
        &self,
        context: &MutationContext,
        command: UpdateSubscriptionPlan,
    ) -> Result<SubscriptionPlanMutation, AdminError>;

    async fn set_plan_enabled(
        &self,
        context: &MutationContext,
        command: SetSubscriptionPlanEnabled,
    ) -> Result<SubscriptionPlanMutation, AdminError>;
    async fn current_subscription(
        &self,
        user_id: &str,
    ) -> Result<Option<UserSubscriptionRecord>, AdminError>;
    async fn grant_subscription(
        &self,
        context: &MutationContext,
        command: CreateUserSubscription,
    ) -> Result<UserSubscriptionMutation, AdminError>;
    async fn revoke_subscription(
        &self,
        context: &MutationContext,
        user_id: &str,
    ) -> Result<Option<UserSubscriptionMutation>, AdminError>;
    async fn update_subscription(
        &self,
        context: &MutationContext,
        command: UpdateUserSubscription,
    ) -> Result<UserSubscriptionMutation, AdminError>;
    async fn renew_subscription(
        &self,
        context: &MutationContext,
        command: RenewUserSubscription,
    ) -> Result<UserSubscriptionMutation, AdminError>;
    async fn subscription_history(
        &self,
        user_id: &str,
        pagination: ControlPageQuery,
    ) -> Result<ControlPage<UserSubscriptionRecord>, AdminError>;
    async fn user_groups(&self, user_id: &str) -> Result<UserGroups, AdminError>;
    async fn replace_user_groups(
        &self,
        context: &MutationContext,
        user_id: &str,
        group_ids: Vec<gateway_core::routing::AccountGroupId>,
    ) -> Result<UserGroups, AdminError>;
    async fn add_budget_credit(
        &self,
        context: &MutationContext,
        command: CreateUserBudgetCredit,
    ) -> Result<UserBudgetCreditRecord, AdminError>;
    async fn reset_budget(
        &self,
        context: &MutationContext,
        command: ResetUserBudget,
    ) -> Result<ResetUserBudget, AdminError>;
    async fn budget_credits(
        &self,
        user_id: &str,
        window: Option<BudgetWindowKind>,
        pagination: ControlPageQuery,
    ) -> Result<ControlPage<UserBudgetCreditRecord>, AdminError>;
    async fn management(
        &self,
        query: UserManagementQuery,
    ) -> Result<ControlPage<UserManagementRecord>, AdminError>;
    async fn user_summary(&self, user_id: &str) -> Result<UserBillingSummary, AdminError>;
}

pub(crate) struct DefaultSubscriptionBillingService {
    store: Option<Arc<dyn SubscriptionBillingStore>>,
    snapshot: Arc<dyn SnapshotControl>,
}

impl DefaultSubscriptionBillingService {
    #[must_use]
    pub(crate) fn new(
        store: Option<Arc<dyn SubscriptionBillingStore>>,
        snapshot: Arc<dyn SnapshotControl>,
    ) -> Self {
        Self { store, snapshot }
    }

    fn store(&self) -> Result<&dyn SubscriptionBillingStore, AdminError> {
        self.store
            .as_deref()
            .ok_or_else(|| AdminError::unavailable("订阅计费能力暂不可用"))
    }
}

#[async_trait]
impl SubscriptionBillingService for DefaultSubscriptionBillingService {
    async fn plans(&self) -> Result<Vec<SubscriptionPlanRecord>, AdminError> {
        self.store()?
            .list_subscription_plans()
            .await
            .map_err(|error| map_store_error(error, "subscription plan"))
    }

    async fn create_plan(
        &self,
        context: &MutationContext,
        command: CreateSubscriptionPlan,
    ) -> Result<SubscriptionPlanMutation, AdminError> {
        validate_plan_name(&command.name, command.description.as_deref())?;
        let mutation = self
            .store()?
            .create_subscription_plan(
                NewSubscriptionPlan {
                    id: format!("plan_{}", Uuid::now_v7().simple()),
                    name: command.name,
                    description: command.description,
                    budget_limits: command.budget_limits,
                },
                context,
            )
            .await
            .map_err(|error| map_store_error(error, "subscription plan"))?;
        publish_committed(self.snapshot.as_ref(), mutation.config_revision).await?;
        Ok(mutation)
    }

    async fn update_plan(
        &self,
        context: &MutationContext,
        command: UpdateSubscriptionPlan,
    ) -> Result<SubscriptionPlanMutation, AdminError> {
        validate_plan_name(&command.name, command.description.as_deref())?;
        let mutation = self
            .store()?
            .update_subscription_plan(command, context)
            .await
            .map_err(|error| map_store_error(error, "subscription plan"))?;
        publish_committed(self.snapshot.as_ref(), mutation.config_revision).await?;
        Ok(mutation)
    }

    async fn set_plan_enabled(
        &self,
        context: &MutationContext,
        command: SetSubscriptionPlanEnabled,
    ) -> Result<SubscriptionPlanMutation, AdminError> {
        let mutation = self
            .store()?
            .set_subscription_plan_enabled(command, context)
            .await
            .map_err(|error| map_store_error(error, "subscription plan"))?;
        publish_committed(self.snapshot.as_ref(), mutation.config_revision).await?;
        Ok(mutation)
    }

    async fn current_subscription(
        &self,
        user_id: &str,
    ) -> Result<Option<UserSubscriptionRecord>, AdminError> {
        self.store()?
            .current_user_subscription(user_id)
            .await
            .map_err(|error| map_store_error(error, "user subscription"))
    }

    async fn grant_subscription(
        &self,
        context: &MutationContext,
        command: CreateUserSubscription,
    ) -> Result<UserSubscriptionMutation, AdminError> {
        validate_subscription_dates(command.starts_at, command.expires_at)?;
        let mutation = self
            .store()?
            .grant_user_subscription(
                GrantUserSubscription {
                    id: format!("sub_{}", Uuid::now_v7().simple()),
                    user_id: command.user_id,
                    plan_id: command.plan_id,
                    starts_at: command.starts_at,
                    expires_at: command.expires_at,
                    downstream_rate_multiplier: command.downstream_rate_multiplier,
                },
                context,
            )
            .await
            .map_err(|error| map_store_error(error, "user subscription"))?;
        publish_committed(self.snapshot.as_ref(), mutation.config_revision).await?;
        Ok(mutation)
    }

    async fn revoke_subscription(
        &self,
        context: &MutationContext,
        user_id: &str,
    ) -> Result<Option<UserSubscriptionMutation>, AdminError> {
        let mutation = self
            .store()?
            .revoke_user_subscription(user_id, context)
            .await
            .map_err(|error| map_store_error(error, "user subscription"))?;
        if let Some(mutation) = &mutation {
            publish_committed(self.snapshot.as_ref(), mutation.config_revision).await?;
        }
        Ok(mutation)
    }

    async fn update_subscription(
        &self,
        context: &MutationContext,
        command: UpdateUserSubscription,
    ) -> Result<UserSubscriptionMutation, AdminError> {
        if command.starts_at.is_none()
            && command.expires_at.is_none()
            && command.downstream_rate_multiplier.is_none()
        {
            return Err(AdminError::invalid("至少更新一项订阅配置"));
        }
        if let (Some(start), Some(end)) = (command.starts_at, command.expires_at) {
            validate_subscription_dates(start, end)?;
        }
        let mutation = self
            .store()?
            .update_user_subscription(command, context)
            .await
            .map_err(|error| map_store_error(error, "user subscription"))?;
        publish_committed(self.snapshot.as_ref(), mutation.config_revision).await?;
        Ok(mutation)
    }

    async fn renew_subscription(
        &self,
        context: &MutationContext,
        command: RenewUserSubscription,
    ) -> Result<UserSubscriptionMutation, AdminError> {
        if let SubscriptionRenewal::ExtendByDays(days) = command.renewal
            && (days == 0 || i64::from(days) * 86400 > MAX_SUBSCRIPTION_DURATION_SECONDS)
        {
            return Err(AdminError::invalid("续期天数不合法"));
        }
        let mutation = self
            .store()?
            .renew_user_subscription(command, context)
            .await
            .map_err(|error| map_store_error(error, "user subscription"))?;
        publish_committed(self.snapshot.as_ref(), mutation.config_revision).await?;
        Ok(mutation)
    }

    async fn subscription_history(
        &self,
        user_id: &str,
        pagination: ControlPageQuery,
    ) -> Result<ControlPage<UserSubscriptionRecord>, AdminError> {
        validate_pagination(pagination)?;
        self.store()?
            .list_user_subscription_history(user_id, pagination)
            .await
            .map_err(|error| map_store_error(error, "user subscription"))
    }

    async fn user_groups(&self, user_id: &str) -> Result<UserGroups, AdminError> {
        self.store()?
            .list_user_groups(user_id)
            .await
            .map_err(|error| map_store_error(error, "user groups"))
    }

    async fn replace_user_groups(
        &self,
        context: &MutationContext,
        user_id: &str,
        mut group_ids: Vec<gateway_core::routing::AccountGroupId>,
    ) -> Result<UserGroups, AdminError> {
        group_ids.sort();
        group_ids.dedup();
        let result = self
            .store()?
            .replace_user_groups(user_id, group_ids, context)
            .await
            .map_err(|error| map_store_error(error, "user groups"))?;
        publish_committed(self.snapshot.as_ref(), result.revision).await?;
        Ok(result)
    }

    async fn add_budget_credit(
        &self,
        context: &MutationContext,
        mut command: CreateUserBudgetCredit,
    ) -> Result<UserBudgetCreditRecord, AdminError> {
        command.reason = command.reason.trim().to_owned();
        if command.idempotency_key.trim().is_empty()
            || command.idempotency_key.len() > 128
            || command.idempotency_key.chars().any(char::is_control)
            || command.reason.trim().is_empty()
            || command.reason.chars().count() > 1024
            || command.reason.chars().any(char::is_control)
        {
            return Err(AdminError::invalid("充值幂等键或备注不合法"));
        }
        let (revision, record) = self
            .store()?
            .add_user_budget_credit(
                AddUserBudgetCredit {
                    id: format!("credit_{}", Uuid::now_v7().simple()),
                    user_id: command.user_id,
                    window: command.window,
                    amount: command.amount,
                    idempotency_key: command.idempotency_key,
                    reason: command.reason,
                },
                context,
            )
            .await
            .map_err(|error| map_store_error(error, "user budget credit"))?;
        publish_committed(self.snapshot.as_ref(), revision).await?;
        Ok(record)
    }

    async fn reset_budget(
        &self,
        context: &MutationContext,
        command: ResetUserBudget,
    ) -> Result<ResetUserBudget, AdminError> {
        if !command.daily && !command.weekly && !command.monthly {
            return Err(AdminError::invalid("至少选择一个额度窗口"));
        }
        let revision = self
            .store()?
            .reset_user_budget(command.clone(), context)
            .await
            .map_err(|error| map_store_error(error, "user budget"))?;
        publish_committed(self.snapshot.as_ref(), revision).await?;
        Ok(command)
    }

    async fn budget_credits(
        &self,
        user_id: &str,
        window: Option<BudgetWindowKind>,
        pagination: ControlPageQuery,
    ) -> Result<ControlPage<UserBudgetCreditRecord>, AdminError> {
        validate_pagination(pagination)?;
        self.store()?
            .list_user_budget_credits(user_id, window, pagination)
            .await
            .map_err(|error| map_store_error(error, "user budget credit"))
    }

    async fn management(
        &self,
        query: UserManagementQuery,
    ) -> Result<ControlPage<UserManagementRecord>, AdminError> {
        validate_pagination(query.pagination)?;
        self.store()?
            .list_user_management(query)
            .await
            .map_err(|error| map_store_error(error, "user management"))
    }

    async fn user_summary(&self, user_id: &str) -> Result<UserBillingSummary, AdminError> {
        self.store()?
            .user_billing_summary(user_id)
            .await
            .map_err(|error| map_store_error(error, "user billing summary"))
    }
}

fn validate_plan_name(name: &str, description: Option<&str>) -> Result<(), AdminError> {
    if name.trim().is_empty()
        || name != name.trim()
        || name.len() > 100
        || name.chars().any(char::is_control)
        || description
            .is_some_and(|value| value.len() > 4096 || value.chars().any(char::is_control))
    {
        return Err(AdminError::invalid("套餐配置不合法"));
    }
    Ok(())
}

fn validate_subscription_dates(
    start: chrono::DateTime<chrono::Utc>,
    end: chrono::DateTime<chrono::Utc>,
) -> Result<(), AdminError> {
    let duration = end - start;
    if duration <= chrono::Duration::zero()
        || duration > chrono::Duration::seconds(MAX_SUBSCRIPTION_DURATION_SECONDS)
    {
        return Err(AdminError::invalid(
            "订阅到期必须晚于开始，且期限不能超过一百年",
        ));
    }
    Ok(())
}

fn validate_pagination(query: ControlPageQuery) -> Result<(), AdminError> {
    if query.page == 0 || !(1..=100).contains(&query.page_size) {
        return Err(AdminError::invalid("分页参数不合法"));
    }
    Ok(())
}
