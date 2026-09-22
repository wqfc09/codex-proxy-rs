//! 按 Key 串行检查限额，并幂等累计已取得的 USD 费用。

use std::{
    collections::BTreeMap,
    sync::Mutex,
    time::{Duration, SystemTime},
};

use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use gateway_admin::model::{
    MutationContext,
    client_keys::{ClientKeyBudgetPeriod, ResetClientKeyBudget},
};
use gateway_core::{
    engine::budget::{
        ClientBudgetCharge, ClientBudgetError, ClientBudgetLimits, ClientBudgetPort,
        ClientBudgetStatus, UserBudgetCharge,
    },
    error::{GatewayError, GatewayErrorKind},
    metering::Decimal,
    policy::{ClientApiKeyId, ClientBillingPolicy, UserId},
};
use sqlx::{PgPool, Postgres, Row, Transaction};

use crate::{StoreError, StoreResult, mutation_audit, postgres_unavailable};

pub(super) async fn reset_client_key_budget(
    pool: &PgPool,
    command: ResetClientKeyBudget,
    context: &MutationContext,
    owner_user_id: Option<&str>,
) -> StoreResult<()> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|_| postgres_unavailable("begin budget reset"))?;
    if let Some(user_id) = owner_user_id {
        let enabled =
            sqlx::query_scalar::<_, bool>("select enabled from users where id=$1 for update")
                .bind(user_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|_| postgres_unavailable("lock budget reset owner"))?;
        if enabled != Some(true) {
            return Err(StoreError::NotFound {
                entity: "user",
                id: user_id.to_owned(),
            });
        }
    }
    // 与准入、结算共用 User -> Key 行锁；不能只在事务外检查owner。
    let exists =
        sqlx::query_scalar::<_, String>("select id from client_api_keys where id = $1 and (owner_user_id=$2 or ($2::text is null and owner_user_id is null)) for update")
            .bind(command.id.as_str())
            .bind(owner_user_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|_| postgres_unavailable("lock budget reset key"))?;
    if exists.is_none() {
        return Err(StoreError::NotFound {
            entity: "client API key",
            id: command.id.as_str().to_owned(),
        });
    }
    let daily = matches!(
        command.period,
        ClientKeyBudgetPeriod::Daily | ClientKeyBudgetPeriod::All
    );
    let weekly = matches!(
        command.period,
        ClientKeyBudgetPeriod::Weekly | ClientKeyBudgetPeriod::All
    );
    let reset_at = Utc::now();
    // 推进计费起点，避免重置前完成、稍后落盘的费用重新扣额；未使用的 Key 不开启窗口。
    sqlx::query(
        "update client_key_budget_windows set
        daily_used_usd = case when $2 then 0 else daily_used_usd end,
        daily_start = case when $2 and daily_end > $4 then $4 else daily_start end,
        weekly_used_usd = case when $3 then 0 else weekly_used_usd end,
        weekly_start = case when $3 and weekly_end > $4 then $4 else weekly_start end
        where client_api_key_id = $1",
    )
    .bind(command.id.as_str())
    .bind(daily)
    .bind(weekly)
    .bind(reset_at)
    .execute(&mut *tx)
    .await
    .map_err(|_| postgres_unavailable("reset client budget"))?;
    let mut fields = Vec::new();
    if daily {
        fields.extend(["daily_used_usd".to_owned(), "daily_start".to_owned()]);
    }
    if weekly {
        fields.extend(["weekly_used_usd".to_owned(), "weekly_start".to_owned()]);
    }
    let mut audit = mutation_audit(
        context,
        "reset_budget",
        "client_api_key",
        command.id.as_str(),
        fields,
    );
    if let Some(user_id) = owner_user_id {
        audit.actor_kind = super::AdminAuditActorKind::UserSession;
        audit.actor_admin_user_id = None;
        audit.actor_ref = format!("user:{user_id}");
    }
    super::append_admin_audit_event_in_transaction(&mut tx, audit, None).await?;
    tx.commit()
        .await
        .map_err(|_| postgres_unavailable("commit budget reset"))
}

pub struct PgClientBudgetStore {
    pool: PgPool,
    retry: Mutex<BTreeMap<String, ClientBudgetCharge>>,
    user_retry: Mutex<BTreeMap<String, UserBudgetCharge>>,
}

impl PgClientBudgetStore {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            retry: Mutex::new(BTreeMap::new()),
            user_retry: Mutex::new(BTreeMap::new()),
        }
    }

    async fn admit_inner(&self, key_id: ClientApiKeyId) -> Result<(), GatewayError> {
        // 短暂存储故障后按原金额重试；进程退出丢失的费用不转成人工核账或阻断 Key。
        let retries = self
            .retry
            .lock()
            .map_err(|_| unavailable())?
            .values()
            .filter(|charge| charge.key_id == key_id)
            .cloned()
            .collect::<Vec<_>>();
        for charge in retries {
            self.settle(charge).await.map_err(|_| unavailable())?;
        }
        let mut tx = self.pool.begin().await.map_err(|_| unavailable())?;
        let row = sqlx::query(
            "select daily_limit_usd::text, weekly_limit_usd::text, enabled
            from client_api_keys where id = $1 for update",
        )
        .bind(key_id.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| unavailable())?
        .ok_or_else(|| {
            GatewayError::new(
                GatewayErrorKind::Unauthorized,
                "client API key no longer exists",
            )
        })?;
        if !row.get::<bool, _>("enabled") {
            return Err(GatewayError::new(
                GatewayErrorKind::PolicyDenied,
                "client API key is disabled",
            ));
        }
        let limits = ClientBudgetLimits {
            daily_usd: row
                .get::<String, _>("daily_limit_usd")
                .parse()
                .map_err(|_| unavailable())?,
            weekly_usd: row
                .get::<String, _>("weekly_limit_usd")
                .parse()
                .map_err(|_| unavailable())?,
        };
        let now = Utc::now();
        advance_windows(&mut tx, key_id.as_str(), now)
            .await
            .map_err(|_| unavailable())?;
        if limits.is_limited() {
            let window = sqlx::query(
                "select daily_used_usd::text, weekly_used_usd::text, daily_end, weekly_end
                from client_key_budget_windows where client_api_key_id = $1",
            )
            .bind(key_id.as_str())
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| unavailable())?;
            let daily: Decimal = window
                .get::<String, _>("daily_used_usd")
                .parse()
                .map_err(|_| unavailable())?;
            let weekly: Decimal = window
                .get::<String, _>("weekly_used_usd")
                .parse()
                .map_err(|_| unavailable())?;
            let daily_exceeded = limits.daily_usd != Decimal::ZERO && daily >= limits.daily_usd;
            let weekly_exceeded = limits.weekly_usd != Decimal::ZERO && weekly >= limits.weekly_usd;
            if daily_exceeded || weekly_exceeded {
                let daily_end: DateTime<Utc> = window.get("daily_end");
                let weekly_end: DateTime<Utc> = window.get("weekly_end");
                let reset = if weekly_exceeded {
                    weekly_end
                } else {
                    daily_end
                };
                let retry = (reset - now).to_std().unwrap_or(Duration::from_secs(1));
                return Err(GatewayError::new(
                    GatewayErrorKind::RateLimited,
                    "client API key budget is exhausted",
                )
                .with_client_code(if weekly_exceeded {
                    "key_weekly_budget_exceeded"
                } else {
                    "key_daily_budget_exceeded"
                })
                .with_retry_after(retry));
            }
        }
        tx.commit().await.map_err(|_| unavailable())
    }

    async fn settle_inner(&self, charge: &ClientBudgetCharge) -> Result<(), ClientBudgetError> {
        let mut tx = self.pool.begin().await.map_err(|_| ClientBudgetError)?;
        // 与准入统一先锁 Key，再写窗口和费用，串行化同一 Key 的并发结算。
        let key = sqlx::query_scalar::<_, String>(
            "select id from client_api_keys where id = $1 for update",
        )
        .bind(charge.key_id.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| ClientBudgetError)?;
        let Some(key) = key else { return Ok(()) }; // 删除 Key 时也会删除其费用记录。
        settle_in_transaction(&mut tx, &key, charge)
            .await
            .map_err(|_| ClientBudgetError)?;
        tx.commit().await.map_err(|_| ClientBudgetError)
    }

    async fn admit_user_inner(
        &self,
        user_id: UserId,
        policy: ClientBillingPolicy,
    ) -> Result<(), GatewayError> {
        if !policy.is_active_at(SystemTime::now()) {
            return Err(GatewayError::new(
                GatewayErrorKind::PolicyDenied,
                "user subscription is not active",
            ));
        }
        let retries = self
            .user_retry
            .lock()
            .map_err(|_| unavailable())?
            .values()
            .filter(|charge| charge.user_id == user_id)
            .cloned()
            .collect::<Vec<_>>();
        for charge in retries {
            self.settle_user(charge).await.map_err(|_| unavailable())?;
        }
        let mut tx = self.pool.begin().await.map_err(|_| unavailable())?;
        let enabled =
            sqlx::query_scalar::<_, bool>("select enabled from users where id=$1 for update")
                .bind(user_id.as_str())
                .fetch_optional(&mut *tx)
                .await
                .map_err(|_| unavailable())?
                .ok_or_else(|| {
                    GatewayError::new(GatewayErrorKind::Unauthorized, "user no longer exists")
                })?;
        if !enabled {
            return Err(GatewayError::new(
                GatewayErrorKind::PolicyDenied,
                "user is disabled",
            ));
        }
        advance_user_windows(&mut tx, user_id.as_str(), Utc::now())
            .await
            .map_err(|_| unavailable())?;
        let row = sqlx::query(
            "select daily_used_usd::text, daily_start, daily_end,
                    weekly_used_usd::text, weekly_start, weekly_end,
                    monthly_used_usd::text, monthly_start, monthly_end
             from user_budget_windows where user_id=$1",
        )
        .bind(user_id.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| unavailable())?;
        let daily = parse_decimal_row(&row, "daily_used_usd")?;
        let weekly = parse_decimal_row(&row, "weekly_used_usd")?;
        let monthly = parse_decimal_row(&row, "monthly_used_usd")?;
        // Credits 只属于发放时的 plan/subscription/窗口。精确匹配边界让滚动窗口、套餐切换
        // 和 reset 自然失效旧额度，避免把历史赠额带入新的有效策略。
        let daily_limit = effective_user_budget_limit(
            &mut tx,
            user_id.as_str(),
            &policy,
            policy.budget_limits().daily_usd,
            "daily",
            row.get("daily_start"),
            row.get("daily_end"),
        )
        .await?;
        let weekly_limit = effective_user_budget_limit(
            &mut tx,
            user_id.as_str(),
            &policy,
            policy.budget_limits().weekly_usd,
            "weekly",
            row.get("weekly_start"),
            row.get("weekly_end"),
        )
        .await?;
        let monthly_limit = effective_user_budget_limit(
            &mut tx,
            user_id.as_str(),
            &policy,
            policy.budget_limits().monthly_usd,
            "monthly",
            row.get("monthly_start"),
            row.get("monthly_end"),
        )
        .await?;
        let daily_exceeded = daily_limit.is_some_and(|limit| daily >= limit);
        let weekly_exceeded = weekly_limit.is_some_and(|limit| weekly >= limit);
        let monthly_exceeded = monthly_limit.is_some_and(|limit| monthly >= limit);
        if daily_exceeded || weekly_exceeded || monthly_exceeded {
            let mut reset = row.get::<DateTime<Utc>, _>("daily_end");
            let mut code = "user_daily_budget_exceeded";
            if weekly_exceeded {
                reset = row.get("weekly_end");
                code = "user_weekly_budget_exceeded";
            }
            if monthly_exceeded {
                reset = row.get("monthly_end");
                code = "user_monthly_budget_exceeded";
            }
            return Err(GatewayError::new(
                GatewayErrorKind::RateLimited,
                "user plan budget is exhausted",
            )
            .with_client_code(code)
            .with_retry_after(
                (reset - Utc::now())
                    .to_std()
                    .unwrap_or(Duration::from_secs(1)),
            ));
        }
        tx.commit().await.map_err(|_| unavailable())
    }

    async fn settle_user_inner(&self, charge: &UserBudgetCharge) -> Result<(), ClientBudgetError> {
        let Some(base) = charge.base_amount_usd else {
            return Ok(());
        };
        let billed = base
            .checked_mul(charge.policy.downstream_rate_multiplier())
            .ok_or(ClientBudgetError)?;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|error| user_settlement_error("begin", error))?;
        let exists = sqlx::query_scalar::<_, String>("select id from users where id=$1 for update")
            .bind(charge.user_id.as_str())
            .fetch_optional(&mut *tx)
            .await
            .map_err(|error| user_settlement_error("lock user", error))?;
        let Some(_) = exists else { return Ok(()) };
        // 与用户准入的 User -> Key 锁序一致，避免结算和控制面反向等待。
        let key_exists = sqlx::query_scalar::<_, String>(
            "select id from client_api_keys where id=$1 for update",
        )
        .bind(charge.key_id.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(|error| user_settlement_error("lock key", error))?;
        // 费用属于完成时间所在窗口；排队落盘或重试不能提前滚到处理时刻。
        // 已由后续请求推进的窗口保持单调，不把旧费用扣入新日窗口。
        let completed_at = DateTime::<Utc>::from(charge.completed_at);
        advance_user_windows(&mut tx, charge.user_id.as_str(), completed_at)
            .await
            .map_err(|error| user_settlement_error("advance user window", error))?;
        if key_exists.is_some() {
            advance_windows(&mut tx, charge.key_id.as_str(), completed_at)
                .await
                .map_err(|error| user_settlement_error("advance key window", error))?;
        }
        let inserted = sqlx::query_scalar::<_, String>(
            "insert into user_charge_events
               (request_id,user_id,subscription_id,user_ref,subscription_ref,plan_id,
                billing_group_ref,base_amount_usd,downstream_rate_multiplier,billed_amount_usd,completed_at)
             values ($1,$2,$3,$4,$5,$6,null,$7::text::numeric,$8::text::numeric,$9::text::numeric,$10)
             on conflict (request_id) do nothing
             returning billed_amount_usd::text",
        )
        .bind(charge.request_id.as_str())
        .bind(charge.user_id.as_str())
        .bind(charge.policy.subscription_id().map(|id| id.as_str()))
        .bind(charge.user_id.as_str())
        .bind(charge.policy.subscription_id().map(|id| id.as_str()))
        .bind(charge.policy.plan_id().as_str())
        .bind(base.canonical())
        .bind(charge.policy.downstream_rate_multiplier().canonical())
        .bind(billed.canonical())
        .bind(DateTime::<Utc>::from(charge.completed_at))
        .fetch_optional(&mut *tx)
        .await
        .map_err(|error| user_settlement_error("insert user charge", error))?;
        let (authoritative_billed, inserted_user_charge) = if let Some(value) = inserted {
            (
                value.parse::<Decimal>().map_err(|_| ClientBudgetError)?,
                true,
            )
        } else {
            let value = sqlx::query_scalar::<_, String>(
                "select billed_amount_usd::text from user_charge_events where request_id=$1",
            )
            .bind(charge.request_id.as_str())
            .fetch_one(&mut *tx)
            .await
            .map_err(|error| user_settlement_error("load existing user charge", error))?;
            (
                value.parse::<Decimal>().map_err(|_| ClientBudgetError)?,
                false,
            )
        };
        if inserted_user_charge {
            sqlx::query(
                "update user_budget_windows set
                   daily_used_usd = daily_used_usd + case when $2 >= daily_start and $2 < daily_end then $1::text::numeric else 0 end,
                   weekly_used_usd = weekly_used_usd + case when $2 >= weekly_start and $2 < weekly_end then $1::text::numeric else 0 end,
                   monthly_used_usd = monthly_used_usd + case when $2 >= monthly_start and $2 < monthly_end then $1::text::numeric else 0 end
                 where user_id=$3",
            )
            .bind(authoritative_billed.canonical())
            .bind(DateTime::<Utc>::from(charge.completed_at))
            .bind(charge.user_id.as_str())
            .execute(&mut *tx)
            .await
            .map_err(|error| user_settlement_error("update user window", error))?;
        }
        if key_exists.is_some() {
            let key_inserted = sqlx::query(
                "insert into client_key_charge_events
                   (request_id,client_api_key_id,amount_usd,completed_at)
                 values ($1,$2,$3::text::numeric,$4)
                 on conflict (request_id) do nothing",
            )
            .bind(charge.request_id.as_str())
            .bind(charge.key_id.as_str())
            .bind(authoritative_billed.canonical())
            .bind(DateTime::<Utc>::from(charge.completed_at))
            .execute(&mut *tx)
            .await
            .map_err(|error| user_settlement_error("insert key charge", error))?
            .rows_affected();
            if key_inserted == 1 {
                sqlx::query(
                    "update client_key_budget_windows set
                       daily_used_usd = daily_used_usd + case when $2 >= daily_start and $2 < daily_end then $1::text::numeric else 0 end,
                       weekly_used_usd = weekly_used_usd + case when $2 >= weekly_start and $2 < weekly_end then $1::text::numeric else 0 end
                     where client_api_key_id=$3",
                )
                .bind(authoritative_billed.canonical())
                .bind(DateTime::<Utc>::from(charge.completed_at))
                .bind(charge.key_id.as_str())
                .execute(&mut *tx)
                .await
                .map_err(|error| user_settlement_error("update key window", error))?;
            }
        }
        sqlx::query(
            "update model_requests
             set downstream_billed_amount = coalesce(downstream_billed_amount, $1::text::numeric)
             where id=$2",
        )
        .bind(authoritative_billed.canonical())
        .bind(charge.request_id.as_str())
        .execute(&mut *tx)
        .await
        .map_err(|error| user_settlement_error("update request billing", error))?;
        tx.commit()
            .await
            .map_err(|error| user_settlement_error("commit", error))
    }
}

async fn effective_user_budget_limit(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
    policy: &ClientBillingPolicy,
    limit: Option<Decimal>,
    window_kind: &str,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Result<Option<Decimal>, GatewayError> {
    let Some(limit) = limit else {
        return Ok(None);
    };
    let credit = sqlx::query_scalar::<_, String>(
        "select coalesce(sum(amount_usd), 0)::text
         from user_budget_credits
         where user_id = $1 and plan_id = $2
           and subscription_id is not distinct from $3
           and window_kind = $4 and window_start = $5 and window_end = $6",
    )
    .bind(user_id)
    .bind(policy.plan_id().as_str())
    .bind(policy.subscription_id().map(|id| id.as_str()))
    .bind(window_kind)
    .bind(window_start)
    .bind(window_end)
    .fetch_one(&mut **tx)
    .await
    .map_err(|_| unavailable())?
    .parse::<Decimal>()
    .map_err(|_| unavailable())?;
    limit.checked_add(credit).ok_or_else(unavailable).map(Some)
}

async fn settle_in_transaction(
    tx: &mut Transaction<'_, Postgres>,
    key: &str,
    charge: &ClientBudgetCharge,
) -> Result<(), sqlx::Error> {
    advance_windows(tx, key, DateTime::<Utc>::from(charge.completed_at)).await?;
    // 仅在请求结束时写入费用；请求 ID 冲突时不重复累计。
    let changed = sqlx::query(
        "insert into client_key_charge_events (request_id, client_api_key_id, amount_usd, completed_at)
            values ($1, $2, $3::text::numeric, $4)
            on conflict (request_id) do nothing",
    )
    .bind(charge.request_id.as_str())
    .bind(key)
    .bind(charge.amount_usd.canonical())
    .bind(DateTime::<Utc>::from(charge.completed_at))
    .execute(&mut **tx)
    .await?
    .rows_affected();
    if changed == 1 {
        sqlx::query("update client_key_budget_windows set
                daily_used_usd = daily_used_usd + case when $3 >= daily_start and $3 < daily_end then $2::text::numeric else 0 end,
                weekly_used_usd = weekly_used_usd + case when $3 >= weekly_start and $3 < weekly_end then $2::text::numeric else 0 end
                where client_api_key_id = $1")
                .bind(key).bind(charge.amount_usd.canonical()).bind(DateTime::<Utc>::from(charge.completed_at))
                .execute(&mut **tx).await?;
    }
    Ok(())
}

impl ClientBudgetPort for PgClientBudgetStore {
    fn admit(&self, key_id: ClientApiKeyId) -> BoxFuture<'_, Result<(), GatewayError>> {
        Box::pin(async move { self.admit_inner(key_id).await })
    }

    fn settle(&self, charge: ClientBudgetCharge) -> BoxFuture<'_, Result<(), ClientBudgetError>> {
        Box::pin(async move {
            let result = self.settle_inner(&charge).await;
            let mut retry = self.retry.lock().map_err(|_| ClientBudgetError)?;
            if result.is_err() {
                retry.insert(charge.request_id.as_str().to_owned(), charge);
            } else {
                retry.remove(charge.request_id.as_str());
            }
            result
        })
    }

    fn admit_user(
        &self,
        user_id: UserId,
        policy: ClientBillingPolicy,
    ) -> BoxFuture<'_, Result<(), GatewayError>> {
        Box::pin(async move { self.admit_user_inner(user_id, policy).await })
    }

    fn settle_user(
        &self,
        charge: UserBudgetCharge,
    ) -> BoxFuture<'_, Result<(), ClientBudgetError>> {
        Box::pin(async move {
            let result = self.settle_user_inner(&charge).await;
            let mut retry = self.user_retry.lock().map_err(|_| ClientBudgetError)?;
            if result.is_err() {
                retry.insert(charge.request_id.as_str().to_owned(), charge);
            } else {
                retry.remove(charge.request_id.as_str());
            }
            result
        })
    }
}

fn parse_decimal_row(row: &sqlx::postgres::PgRow, column: &str) -> Result<Decimal, GatewayError> {
    row.get::<String, _>(column)
        .parse()
        .map_err(|_| unavailable())
}

pub(crate) async fn advance_user_windows(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &str,
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "insert into user_budget_windows
           (user_id,daily_start,daily_end,weekly_start,weekly_end,monthly_start,monthly_end)
         select $1, day, day + interval '24 hours', week, week + interval '168 hours', month, month + interval '1 month'
         from (
           select date_trunc('day',$2::timestamptz at time zone 'Asia/Shanghai') at time zone 'Asia/Shanghai' as day,
                  date_trunc('week',$2::timestamptz at time zone 'Asia/Shanghai') at time zone 'Asia/Shanghai' as week,
                  date_trunc('month',$2::timestamptz at time zone 'Asia/Shanghai') at time zone 'Asia/Shanghai' as month
         ) d
         on conflict (user_id) do update set
           daily_start = case when user_budget_windows.daily_end <= $2 then excluded.daily_start else user_budget_windows.daily_start end,
           daily_end = case when user_budget_windows.daily_end <= $2 then excluded.daily_end else user_budget_windows.daily_end end,
           daily_used_usd = case when user_budget_windows.daily_end <= $2 then 0 else user_budget_windows.daily_used_usd end,
           weekly_start = case when user_budget_windows.weekly_end <= $2 then excluded.weekly_start else user_budget_windows.weekly_start end,
           weekly_end = case when user_budget_windows.weekly_end <= $2 then excluded.weekly_end else user_budget_windows.weekly_end end,
           weekly_used_usd = case when user_budget_windows.weekly_end <= $2 then 0 else user_budget_windows.weekly_used_usd end,
           monthly_start = case when user_budget_windows.monthly_end <= $2 then excluded.monthly_start else user_budget_windows.monthly_start end,
           monthly_end = case when user_budget_windows.monthly_end <= $2 then excluded.monthly_end else user_budget_windows.monthly_end end,
           monthly_used_usd = case when user_budget_windows.monthly_end <= $2 then 0 else user_budget_windows.monthly_used_usd end",
    )
    .bind(user_id)
    .bind(now)
    .execute(&mut **tx)
    .await
    .map(|_| ())
}

pub(crate) async fn advance_windows(
    tx: &mut Transaction<'_, Postgres>,
    key: &str,
    now: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query("insert into client_key_budget_windows
        (client_api_key_id, daily_start, daily_end, weekly_start, weekly_end)
        select $1, day, day + interval '24 hours', day, day + interval '168 hours'
        from (select date_trunc('day', $2::timestamptz at time zone 'Asia/Shanghai') at time zone 'Asia/Shanghai' as day) d
        on conflict (client_api_key_id) do update set
            daily_start = case when client_key_budget_windows.daily_end <= $2 then excluded.daily_start else client_key_budget_windows.daily_start end,
            daily_end = case when client_key_budget_windows.daily_end <= $2 then excluded.daily_end else client_key_budget_windows.daily_end end,
            daily_used_usd = case when client_key_budget_windows.daily_end <= $2 then 0 else client_key_budget_windows.daily_used_usd end,
            weekly_start = case when client_key_budget_windows.weekly_end <= $2 then excluded.weekly_start else client_key_budget_windows.weekly_start end,
            weekly_end = case when client_key_budget_windows.weekly_end <= $2 then excluded.weekly_end else client_key_budget_windows.weekly_end end,
            weekly_used_usd = case when client_key_budget_windows.weekly_end <= $2 then 0 else client_key_budget_windows.weekly_used_usd end")
        .bind(key).bind(now).execute(&mut **tx).await?;
    Ok(())
}

pub(super) async fn load_client_key_budgets(
    pool: &PgPool,
    records: &mut [super::ClientApiKeyRecord],
) -> StoreResult<()> {
    if records.is_empty() {
        return Ok(());
    }
    let ids = records
        .iter()
        .map(|record| record.id.as_str())
        .collect::<Vec<_>>();
    let rows = sqlx::query(
        "select k.id, k.daily_limit_usd::text, k.weekly_limit_usd::text,
        (case when w.daily_end > now() then w.daily_used_usd else 0 end)::text as daily_used,
        (case when w.weekly_end > now() then w.weekly_used_usd else 0 end)::text as weekly_used,
        case when w.daily_end > now() then w.daily_end end as daily_end,
        case when w.weekly_end > now() then w.weekly_end end as weekly_end
        from client_api_keys k left join client_key_budget_windows w on w.client_api_key_id = k.id
        where k.id = any($1)",
    )
    .bind(ids)
    .fetch_all(pool)
    .await
    .map_err(|_| postgres_unavailable("load client budgets"))?;
    let mut budgets = BTreeMap::new();
    for row in rows {
        let parse = |field| -> StoreResult<Decimal> {
            row.get::<String, _>(field)
                .parse()
                .map_err(|_| postgres_unavailable("decode client budget"))
        };
        budgets.insert(
            row.get::<String, _>("id"),
            ClientBudgetStatus {
                limits: ClientBudgetLimits {
                    daily_usd: parse("daily_limit_usd")?,
                    weekly_usd: parse("weekly_limit_usd")?,
                },
                daily_used_usd: parse("daily_used")?,
                weekly_used_usd: parse("weekly_used")?,
                daily_resets_at: row
                    .get::<Option<DateTime<Utc>>, _>("daily_end")
                    .map(Into::into),
                weekly_resets_at: row
                    .get::<Option<DateTime<Utc>>, _>("weekly_end")
                    .map(Into::into),
            },
        );
    }
    for record in records {
        record.budget = budgets
            .remove(&record.id)
            .ok_or_else(|| postgres_unavailable("load client budget policy"))?;
    }
    Ok(())
}

fn unavailable() -> GatewayError {
    GatewayError::new(
        GatewayErrorKind::ProviderInfrastructureUnavailable,
        "client budget service is temporarily unavailable",
    )
    .with_client_code("key_budget_unavailable")
}

fn user_settlement_error(stage: &'static str, error: sqlx::Error) -> ClientBudgetError {
    tracing::error!(stage, error = %error, "user budget settlement SQL failed");
    ClientBudgetError
}
