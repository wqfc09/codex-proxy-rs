mod mutations;
mod reads;

use async_trait::async_trait;
use gateway_admin::{
    model::{MutationContext, Revision, subscription_billing::*},
    ports::store::{
        AdminStoreError, AdminStoreErrorKind, AdminStoreResult, SubscriptionBillingStore,
    },
};
use gateway_core::{metering::Decimal, routing::AccountGroupId};
use sqlx::{PgPool, Postgres, Transaction};

use super::{append_admin_audit_event_in_transaction, bump_config_revision_in_transaction};
use crate::{StoreError, admin_revision, admin_store_error, mutation_audit, postgres_unavailable};

const ENTITY: &str = "subscription billing";

#[derive(Clone)]
pub struct PgSubscriptionBillingStore {
    pool: PgPool,
}

impl PgSubscriptionBillingStore {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

async fn finish(
    mut tx: Transaction<'_, Postgres>,
    context: &MutationContext,
    action: &str,
    kind: &str,
    id: &str,
    fields: Vec<String>,
) -> AdminStoreResult<Revision> {
    let revision = bump_config_revision_in_transaction(&mut tx)
        .await
        .map_err(|e| admin_store_error(ENTITY, e))?;
    append_admin_audit_event_in_transaction(
        &mut tx,
        mutation_audit(context, action, kind, id, fields),
        revision,
    )
    .await
    .map_err(|e| admin_store_error(ENTITY, e))?;
    tx.commit()
        .await
        .map_err(|_| unavailable("commit billing mutation"))?;
    admin_revision(revision)
}

async fn lock_user(tx: &mut Transaction<'_, Postgres>, user_id: &str) -> AdminStoreResult<()> {
    sqlx::query_scalar::<_, String>("select id from users where id=$1 for update")
        .bind(user_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| unavailable("lock billing user"))?
        .ok_or_else(|| not_found("user", user_id))?;
    Ok(())
}

async fn begin_read(pool: &PgPool) -> AdminStoreResult<Transaction<'_, Postgres>> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|_| unavailable("begin billing read"))?;
    sqlx::query("set transaction isolation level repeatable read, read only")
        .execute(&mut *tx)
        .await
        .map_err(|_| unavailable("set billing read snapshot"))?;
    Ok(tx)
}

fn conflict(message: &'static str) -> AdminStoreError {
    AdminStoreError::new(AdminStoreErrorKind::Conflict, ENTITY, message)
}

#[async_trait]
impl SubscriptionBillingStore for PgSubscriptionBillingStore {
    async fn list_subscription_plans(&self) -> AdminStoreResult<Vec<SubscriptionPlanRecord>> {
        let mut conn = self
            .pool
            .acquire()
            .await
            .map_err(|_| unavailable("acquire plans connection"))?;
        reads::plans(&mut conn).await
    }
    async fn create_subscription_plan(
        &self,
        command: NewSubscriptionPlan,
        context: &MutationContext,
    ) -> AdminStoreResult<SubscriptionPlanMutation> {
        mutations::create_plan(&self.pool, command, context).await
    }
    async fn update_subscription_plan(
        &self,
        command: UpdateSubscriptionPlan,
        context: &MutationContext,
    ) -> AdminStoreResult<SubscriptionPlanMutation> {
        mutations::update_plan(&self.pool, command, context).await
    }
    async fn set_subscription_plan_enabled(
        &self,
        command: SetSubscriptionPlanEnabled,
        context: &MutationContext,
    ) -> AdminStoreResult<SubscriptionPlanMutation> {
        mutations::set_plan_enabled(&self.pool, command, context).await
    }
    async fn current_user_subscription(
        &self,
        user_id: &str,
    ) -> AdminStoreResult<Option<UserSubscriptionRecord>> {
        Ok(self.user_billing_summary(user_id).await?.subscription)
    }
    async fn grant_user_subscription(
        &self,
        command: GrantUserSubscription,
        context: &MutationContext,
    ) -> AdminStoreResult<UserSubscriptionMutation> {
        mutations::grant(&self.pool, command, context).await
    }
    async fn update_user_subscription(
        &self,
        command: UpdateUserSubscription,
        context: &MutationContext,
    ) -> AdminStoreResult<UserSubscriptionMutation> {
        mutations::update_subscription(&self.pool, command, context).await
    }
    async fn renew_user_subscription(
        &self,
        command: RenewUserSubscription,
        context: &MutationContext,
    ) -> AdminStoreResult<UserSubscriptionMutation> {
        mutations::renew(&self.pool, command, context).await
    }
    async fn revoke_user_subscription(
        &self,
        user_id: &str,
        context: &MutationContext,
    ) -> AdminStoreResult<Option<UserSubscriptionMutation>> {
        mutations::revoke(&self.pool, user_id, context).await
    }
    async fn user_billing_summary(&self, user_id: &str) -> AdminStoreResult<UserBillingSummary> {
        let mut tx = begin_read(&self.pool).await?;
        reads::summaries(&mut tx, &[user_id.to_owned()])
            .await?
            .remove(user_id)
            .ok_or_else(|| not_found("user", user_id))
    }
    async fn list_user_subscription_history(
        &self,
        user_id: &str,
        pagination: ControlPageQuery,
    ) -> AdminStoreResult<ControlPage<UserSubscriptionRecord>> {
        reads::history(&self.pool, user_id, pagination).await
    }
    async fn list_user_groups(&self, user_id: &str) -> AdminStoreResult<UserGroups> {
        let mut tx = begin_read(&self.pool).await?;
        reads::user_groups(&mut tx, user_id).await
    }
    async fn replace_user_groups(
        &self,
        user_id: &str,
        group_ids: Vec<AccountGroupId>,
        context: &MutationContext,
    ) -> AdminStoreResult<UserGroups> {
        mutations::groups(&self.pool, user_id, group_ids, context).await
    }
    async fn add_user_budget_credit(
        &self,
        command: AddUserBudgetCredit,
        context: &MutationContext,
    ) -> AdminStoreResult<(Revision, UserBudgetCreditRecord)> {
        mutations::topup(&self.pool, command, context).await
    }
    async fn reset_user_budget(
        &self,
        command: ResetUserBudget,
        context: &MutationContext,
    ) -> AdminStoreResult<Revision> {
        mutations::reset_budget(&self.pool, command, context).await
    }
    async fn list_user_budget_credits(
        &self,
        user_id: &str,
        window: Option<BudgetWindowKind>,
        pagination: ControlPageQuery,
    ) -> AdminStoreResult<ControlPage<UserBudgetCreditRecord>> {
        reads::credits(&self.pool, user_id, window, pagination).await
    }
    async fn list_user_management(
        &self,
        query: UserManagementQuery,
    ) -> AdminStoreResult<ControlPage<UserManagementRecord>> {
        reads::management(&self.pool, query).await
    }
}
fn parse_decimal(value: &str) -> AdminStoreResult<Decimal> {
    value.parse().map_err(|_| invalid("invalid decimal"))
}

fn parse_optional_decimal(value: Option<String>) -> AdminStoreResult<Option<Decimal>> {
    value.map(|value| parse_decimal(&value)).transpose()
}

fn map_subscription_grant_write(error: sqlx::Error, user_id: &str) -> StoreError {
    if error
        .as_database_error()
        .is_some_and(|database| database.is_unique_violation())
    {
        StoreError::Conflict {
            entity: "user subscription",
            id: user_id.to_owned(),
            kind: crate::ConflictKind::InvalidTransition,
        }
    } else {
        postgres_unavailable("grant user subscription")
    }
}

fn map_plan_write(error: sqlx::Error, id: &str) -> StoreError {
    if error
        .as_database_error()
        .is_some_and(|database| database.is_unique_violation())
    {
        StoreError::Conflict {
            entity: "subscription plan",
            id: id.to_owned(),
            kind: crate::ConflictKind::DuplicateName,
        }
    } else {
        postgres_unavailable("write subscription plan")
    }
}

fn unavailable(message: &'static str) -> AdminStoreError {
    admin_store_error(ENTITY, postgres_unavailable(message))
}

fn invalid(message: &'static str) -> AdminStoreError {
    AdminStoreError::new(AdminStoreErrorKind::Invalid, ENTITY, message)
}

fn not_found(resource: &'static str, id: &str) -> AdminStoreError {
    AdminStoreError::new(
        AdminStoreErrorKind::NotFound,
        resource,
        format!("{id} not found"),
    )
}
