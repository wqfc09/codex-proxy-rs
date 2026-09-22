use super::*;
use chrono::{DateTime, Duration, Utc};

fn validate_plan(name: &str, description: Option<&str>) -> AdminStoreResult<()> {
    if name.trim().is_empty()
        || name.trim() != name
        || name.len() > 100
        || name.chars().any(char::is_control)
        || description.is_some_and(|v| v.len() > 4096 || v.chars().any(char::is_control))
    {
        return Err(invalid("invalid plan configuration"));
    }
    Ok(())
}
fn dates(start: DateTime<Utc>, end: DateTime<Utc>) -> AdminStoreResult<()> {
    if end <= start || end - start > Duration::seconds(MAX_SUBSCRIPTION_DURATION_SECONDS) {
        return Err(invalid("invalid subscription interval"));
    }
    Ok(())
}
async fn begin(pool: &PgPool) -> AdminStoreResult<Transaction<'_, Postgres>> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|_| unavailable("begin control mutation"))?;
    // 先锁全局 revision，与其他管理模块顺序一致；用户行锁继续串行化同用户计费事实。
    sqlx::query("select config_revision from runtime_settings where id=1 for update")
        .execute(&mut *tx)
        .await
        .map_err(|_| unavailable("lock control revision"))?;
    Ok(tx)
}

async fn finish_user(
    mut tx: Transaction<'_, Postgres>,
    context: &MutationContext,
    action: &str,
    user: &str,
    fields: Vec<String>,
) -> AdminStoreResult<Revision> {
    sqlx::query("update users set updated_at=greatest(clock_timestamp(),updated_at) where id=$1")
        .bind(user)
        .execute(&mut *tx)
        .await
        .map_err(|_| unavailable("touch billing user"))?;
    finish(tx, context, action, "user", user, fields).await
}
async fn lock_plan(tx: &mut Transaction<'_, Postgres>, id: &str) -> AdminStoreResult<bool> {
    sqlx::query_scalar::<_, bool>("select enabled from subscription_plans where id=$1 for share")
        .bind(id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|_| unavailable("lock subscription plan"))?
        .ok_or_else(|| not_found("subscription plan", id))
}

pub(super) async fn create_plan(
    pool: &PgPool,
    c: NewSubscriptionPlan,
    context: &MutationContext,
) -> AdminStoreResult<SubscriptionPlanMutation> {
    validate_plan(&c.name, c.description.as_deref())?;
    let mut tx = begin(pool).await?;
    sqlx::query(
        "insert into subscription_plans(id,name,description,enabled,
        daily_limit_usd,weekly_limit_usd,monthly_limit_usd,created_at,updated_at)
        values($1,$2,$3,true,$4::text::numeric,$5::text::numeric,$6::text::numeric,now(),now())",
    )
    .bind(&c.id)
    .bind(&c.name)
    .bind(&c.description)
    .bind(c.budget_limits.daily_usd.map(Decimal::canonical))
    .bind(c.budget_limits.weekly_usd.map(Decimal::canonical))
    .bind(c.budget_limits.monthly_usd.map(Decimal::canonical))
    .execute(&mut *tx)
    .await
    .map_err(|e| admin_store_error(ENTITY, map_plan_write(e, &c.id)))?;
    let record = reads::plan(&mut tx, &c.id).await?;
    let config_revision = finish(
        tx,
        context,
        "subscription_plan.create",
        "subscription_plan",
        &c.id,
        vec!["plan".to_owned()],
    )
    .await?;
    Ok(SubscriptionPlanMutation {
        config_revision,
        record,
    })
}

pub(super) async fn update_plan(
    pool: &PgPool,
    c: UpdateSubscriptionPlan,
    context: &MutationContext,
) -> AdminStoreResult<SubscriptionPlanMutation> {
    validate_plan(&c.name, c.description.as_deref())?;
    let mut tx = begin(pool).await?;
    let changed=sqlx::query("update subscription_plans set name=$2,description=$3,
        daily_limit_usd=$4::text::numeric,weekly_limit_usd=$5::text::numeric,monthly_limit_usd=$6::text::numeric,updated_at=now() where id=$1")
        .bind(&c.id).bind(&c.name).bind(&c.description)
        .bind(c.budget_limits.daily_usd.map(Decimal::canonical)).bind(c.budget_limits.weekly_usd.map(Decimal::canonical))
        .bind(c.budget_limits.monthly_usd.map(Decimal::canonical)).execute(&mut *tx).await
        .map_err(|e|admin_store_error(ENTITY,map_plan_write(e,&c.id)))?.rows_affected();
    if changed == 0 {
        return Err(not_found("subscription plan", &c.id));
    }
    let record = reads::plan(&mut tx, &c.id).await?;
    let config_revision = finish(
        tx,
        context,
        "subscription_plan.update",
        "subscription_plan",
        &c.id,
        vec!["plan".to_owned()],
    )
    .await?;
    Ok(SubscriptionPlanMutation {
        config_revision,
        record,
    })
}

pub(super) async fn set_plan_enabled(
    pool: &PgPool,
    c: SetSubscriptionPlanEnabled,
    context: &MutationContext,
) -> AdminStoreResult<SubscriptionPlanMutation> {
    let mut tx = begin(pool).await?;
    let is_base = sqlx::query_scalar::<_, bool>(
        "select is_base from subscription_plans where id=$1 for update",
    )
    .bind(&c.id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|_| unavailable("lock plan state"))?
    .ok_or_else(|| not_found("subscription plan", &c.id))?;
    if is_base && !c.enabled {
        return Err(conflict("base plan cannot be disabled"));
    }
    sqlx::query("update subscription_plans set enabled=$2,updated_at=now() where id=$1")
        .bind(&c.id)
        .bind(c.enabled)
        .execute(&mut *tx)
        .await
        .map_err(|_| unavailable("set plan state"))?;
    let record = reads::plan(&mut tx, &c.id).await?;
    let config_revision = finish(
        tx,
        context,
        if c.enabled {
            "subscription_plan.enable"
        } else {
            "subscription_plan.disable"
        },
        "subscription_plan",
        &c.id,
        vec!["enabled".to_owned()],
    )
    .await?;
    Ok(SubscriptionPlanMutation {
        config_revision,
        record,
    })
}

pub(super) async fn grant(
    pool: &PgPool,
    c: GrantUserSubscription,
    context: &MutationContext,
) -> AdminStoreResult<UserSubscriptionMutation> {
    dates(c.starts_at, c.expires_at)?;
    let mut tx = begin(pool).await?;
    lock_user(&mut tx, &c.user_id).await?;
    if !lock_plan(&mut tx, &c.plan_id).await? {
        return Err(conflict("subscription plan is disabled"));
    }
    sqlx::query("update user_subscriptions set status='revoked',revoked_at=now(),updated_at=now() where user_id=$1 and status='active'")
        .bind(&c.user_id).execute(&mut *tx).await.map_err(|_|unavailable("replace subscription"))?;
    sqlx::query("insert into user_subscriptions(id,user_id,plan_id,status,starts_at,expires_at,downstream_rate_multiplier,created_at,updated_at)
        values($1,$2,$3,'active',$4,$5,$6::text::numeric,now(),now())")
        .bind(&c.id).bind(&c.user_id).bind(&c.plan_id).bind(c.starts_at).bind(c.expires_at).bind(c.downstream_rate_multiplier.canonical())
        .execute(&mut *tx).await.map_err(|e|admin_store_error(ENTITY,map_subscription_grant_write(e,&c.user_id)))?;
    let record = reads::subscription(&mut tx, &c.id).await?;
    let config_revision = finish_user(
        tx,
        context,
        "user_subscription.grant",
        &c.user_id,
        vec![
            "plan_id".to_owned(),
            "starts_at".to_owned(),
            "expires_at".to_owned(),
            "multiplier".to_owned(),
        ],
    )
    .await?;
    Ok(UserSubscriptionMutation {
        record,
        config_revision,
    })
}

pub(super) async fn update_subscription(
    pool: &PgPool,
    c: UpdateUserSubscription,
    context: &MutationContext,
) -> AdminStoreResult<UserSubscriptionMutation> {
    if c.starts_at.is_none() && c.expires_at.is_none() && c.downstream_rate_multiplier.is_none() {
        return Err(invalid("empty subscription update"));
    }
    let mut tx = begin(pool).await?;
    lock_user(&mut tx, &c.user_id).await?;
    let current = reads::unrevoked(&mut tx, &c.user_id)
        .await?
        .ok_or_else(|| conflict("no editable subscription"))?;
    let start = c.starts_at.unwrap_or(current.starts_at);
    let end = c.expires_at.unwrap_or(current.expires_at);
    dates(start, end)?;
    sqlx::query("update user_subscriptions set starts_at=$2,expires_at=$3,downstream_rate_multiplier=$4::text::numeric,updated_at=now() where id=$1")
        .bind(&current.id).bind(start).bind(end).bind(c.downstream_rate_multiplier.unwrap_or(current.downstream_rate_multiplier).canonical())
        .execute(&mut *tx).await.map_err(|_|unavailable("update subscription"))?;
    let record = reads::subscription(&mut tx, &current.id).await?;
    let config_revision = finish_user(
        tx,
        context,
        "user_subscription.update",
        &c.user_id,
        vec![
            "starts_at".to_owned(),
            "expires_at".to_owned(),
            "multiplier".to_owned(),
        ],
    )
    .await?;
    Ok(UserSubscriptionMutation {
        record,
        config_revision,
    })
}

pub(super) async fn renew(
    pool: &PgPool,
    c: RenewUserSubscription,
    context: &MutationContext,
) -> AdminStoreResult<UserSubscriptionMutation> {
    let mut tx = begin(pool).await?;
    lock_user(&mut tx, &c.user_id).await?;
    let current = reads::unrevoked(&mut tx, &c.user_id)
        .await?
        .ok_or_else(|| conflict("no renewable subscription"))?;
    let end = match c.renewal {
        SubscriptionRenewal::ExpiresAt(end) => end,
        SubscriptionRenewal::ExtendByDays(days) => {
            if days == 0 || i64::from(days) * 86400 > MAX_SUBSCRIPTION_DURATION_SECONDS {
                return Err(invalid("invalid renewal duration"));
            }
            current
                .expires_at
                .checked_add_signed(Duration::days(i64::from(days)))
                .ok_or_else(|| invalid("renewal date overflow"))?
        }
    };
    if end <= current.expires_at {
        return Err(invalid("renewal must extend expiry"));
    }
    dates(current.starts_at, end)?;
    sqlx::query("update user_subscriptions set expires_at=$2,updated_at=now() where id=$1")
        .bind(&current.id)
        .bind(end)
        .execute(&mut *tx)
        .await
        .map_err(|_| unavailable("renew subscription"))?;
    let record = reads::subscription(&mut tx, &current.id).await?;
    let config_revision = finish_user(
        tx,
        context,
        "user_subscription.renew",
        &c.user_id,
        vec!["expires_at".to_owned()],
    )
    .await?;
    Ok(UserSubscriptionMutation {
        record,
        config_revision,
    })
}

pub(super) async fn revoke(
    pool: &PgPool,
    user: &str,
    context: &MutationContext,
) -> AdminStoreResult<Option<UserSubscriptionMutation>> {
    let mut tx = begin(pool).await?;
    lock_user(&mut tx, user).await?;
    let Some(current) = reads::unrevoked(&mut tx, user).await? else {
        return Ok(None);
    };
    sqlx::query("update user_subscriptions set status='revoked',revoked_at=now(),updated_at=now() where id=$1")
        .bind(&current.id).execute(&mut *tx).await.map_err(|_|unavailable("revoke subscription"))?;
    let record = reads::subscription(&mut tx, &current.id).await?;
    let config_revision = finish_user(
        tx,
        context,
        "user_subscription.revoke",
        user,
        vec!["status".to_owned(), "revoked_at".to_owned()],
    )
    .await?;
    Ok(Some(UserSubscriptionMutation {
        record,
        config_revision,
    }))
}

pub(super) async fn groups(
    pool: &PgPool,
    user: &str,
    ids: Vec<AccountGroupId>,
    context: &MutationContext,
) -> AdminStoreResult<UserGroups> {
    let mut ids = ids
        .into_iter()
        .map(|g| g.as_str().to_owned())
        .collect::<Vec<_>>();
    ids.sort();
    ids.dedup();
    let mut tx = begin(pool).await?;
    lock_user(&mut tx, user).await?;
    let known = sqlx::query_scalar::<_, String>(
        "select id from account_groups where id=any($1) order by id for key share",
    )
    .bind(&ids)
    .fetch_all(&mut *tx)
    .await
    .map_err(|_| unavailable("validate user groups"))?;
    if known != ids {
        return Err(not_found("account group", "requested group"));
    }
    // 保留仍存在的授权时间，仅删除集合差异；空集合是明确的无授权。
    sqlx::query(
        "delete from user_account_groups where user_id=$1 and not(account_group_id=any($2))",
    )
    .bind(user)
    .bind(&ids)
    .execute(&mut *tx)
    .await
    .map_err(|_| unavailable("remove user groups"))?;
    sqlx::query("insert into user_account_groups(user_id,account_group_id) select $1,unnest($2::text[]) on conflict do nothing")
        .bind(user).bind(&ids).execute(&mut *tx).await.map_err(|_|unavailable("assign user groups"))?;
    let items = reads::user_groups(&mut tx, user).await?.items;
    let revision = finish_user(
        tx,
        context,
        "user.groups.update",
        user,
        vec!["group_ids".to_owned()],
    )
    .await?;
    Ok(UserGroups { items, revision })
}

pub(super) async fn topup(
    pool: &PgPool,
    mut c: AddUserBudgetCredit,
    context: &MutationContext,
) -> AdminStoreResult<(Revision, UserBudgetCreditRecord)> {
    c.reason = c.reason.trim().to_owned();
    if c.reason.is_empty()
        || c.reason.chars().count() > 1024
        || c.reason.chars().any(char::is_control)
        || c.idempotency_key.trim().is_empty()
        || c.idempotency_key.len() > 128
        || c.idempotency_key.chars().any(char::is_control)
    {
        return Err(invalid("invalid topup reason or idempotency key"));
    }
    let mut tx = begin(pool).await?;
    lock_user(&mut tx, &c.user_id).await?;
    // 幂等身份独立于当前策略和时间，必须在查询当前窗口之前处理历史重放。
    if let Some(previous) = reads::credit_by_key(&mut tx, &c.user_id, &c.idempotency_key).await? {
        if previous.window != c.window || previous.amount != c.amount || previous.reason != c.reason
        {
            return Err(conflict(
                "idempotency key already used with another payload",
            ));
        }
        return Ok((reads::revision(&mut tx).await?, previous));
    }
    // 锁住基础与显式候选套餐后再取时钟，排队跨午夜时也只给当前窗口补额。
    sqlx::query("select p.id from subscription_plans p where p.is_base or exists(
        select 1 from user_subscriptions s where s.user_id=$1 and s.status='active' and s.plan_id=p.id)
        order by p.id for share")
        .bind(&c.user_id).fetch_all(&mut *tx).await
        .map_err(|_| unavailable("lock credit policy plans"))?;
    let now = sqlx::query_scalar::<_, DateTime<Utc>>("select clock_timestamp()")
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| unavailable("read topup time"))?;
    super::super::client_budgets::advance_user_windows(&mut tx, &c.user_id, now)
        .await
        .map_err(|_| unavailable("advance credit window"))?;
    let summary = reads::summaries_at(&mut tx, std::slice::from_ref(&c.user_id), now)
        .await?
        .remove(&c.user_id)
        .ok_or_else(|| not_found("user", &c.user_id))?;
    let budget = match c.window {
        BudgetWindowKind::Daily => &summary.budget.daily,
        BudgetWindowKind::Weekly => &summary.budget.weekly,
        BudgetWindowKind::Monthly => &summary.budget.monthly,
    };
    let limit = budget
        .limit
        .ok_or_else(|| conflict("unlimited window needs no credit"))?;
    let credit = budget
        .credit
        .checked_add(c.amount)
        .ok_or_else(|| invalid("credit total overflow"))?;
    limit
        .checked_add(credit)
        .ok_or_else(|| invalid("effective limit overflow"))?;
    let window = reads::windows_at(&mut tx, std::slice::from_ref(&c.user_id), now)
        .await?
        .remove(&c.user_id)
        .and_then(|w| w.into_iter().find(|w| w.kind == c.window))
        .ok_or_else(|| unavailable("credit window missing"))?;
    sqlx::query("insert into user_budget_credits(id,user_id,plan_id,subscription_id,window_kind,window_start,window_end,amount_usd,idempotency_key,reason)
        values($1,$2,$3,$4,$5,$6,$7,$8::text::numeric,$9,$10)")
        .bind(&c.id).bind(&c.user_id).bind(&summary.plan.id).bind(summary.subscription.as_ref().map(|s|s.id.as_str()))
        .bind(c.window.as_str()).bind(window.start).bind(window.end).bind(c.amount.canonical()).bind(&c.idempotency_key).bind(&c.reason)
        .execute(&mut *tx).await.map_err(|_|unavailable("insert budget credit"))?;
    let record = reads::credit_by_id(&mut tx, &c.id).await?;
    let revision = finish_user(
        tx,
        context,
        "user_budget_credit.create",
        &c.user_id,
        vec![format!("credit:{}", c.id), "budget_credit".to_owned()],
    )
    .await?;
    Ok((revision, record))
}

pub(super) async fn reset_budget(
    pool: &PgPool,
    command: ResetUserBudget,
    context: &MutationContext,
) -> AdminStoreResult<Revision> {
    if !command.daily && !command.weekly && !command.monthly {
        return Err(invalid("at least one budget window must be selected"));
    }
    let mut tx = begin(pool).await?;
    lock_user(&mut tx, &command.user_id).await?;
    let now = sqlx::query_scalar::<_, DateTime<Utc>>("select clock_timestamp()")
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| unavailable("read budget reset time"))?;
    super::super::client_budgets::advance_user_windows(&mut tx, &command.user_id, now)
        .await
        .map_err(|_| unavailable("advance budget reset window"))?;

    sqlx::query(
        "update user_budget_windows
         set daily_used_usd = case when $2 then 0 else daily_used_usd end,
             weekly_used_usd = case when $3 then 0 else weekly_used_usd end,
             monthly_used_usd = case when $4 then 0 else monthly_used_usd end
         where user_id = $1",
    )
    .bind(&command.user_id)
    .bind(command.daily)
    .bind(command.weekly)
    .bind(command.monthly)
    .execute(&mut *tx)
    .await
    .map_err(|_| unavailable("reset budget window usage"))?;

    sqlx::query(
        "delete from user_budget_credits c
         using user_budget_windows w
         where c.user_id = $1 and w.user_id = c.user_id
           and (($2 and c.window_kind = 'daily' and c.window_start = w.daily_start and c.window_end = w.daily_end)
             or ($3 and c.window_kind = 'weekly' and c.window_start = w.weekly_start and c.window_end = w.weekly_end)
             or ($4 and c.window_kind = 'monthly' and c.window_start = w.monthly_start and c.window_end = w.monthly_end))",
    )
    .bind(&command.user_id)
    .bind(command.daily)
    .bind(command.weekly)
    .bind(command.monthly)
    .execute(&mut *tx)
    .await
    .map_err(|_| unavailable("clear budget window credits"))?;

    finish_user(
        tx,
        context,
        "user_budget.reset",
        &command.user_id,
        vec![
            "daily".to_owned(),
            "weekly".to_owned(),
            "monthly".to_owned(),
        ],
    )
    .await
}
