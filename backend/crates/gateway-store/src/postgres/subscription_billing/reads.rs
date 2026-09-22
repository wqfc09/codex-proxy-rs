use super::*;
use chrono::{DateTime, Utc};
use gateway_admin::model::users::UserRole;
use gateway_core::policy::UserBudgetLimits;
use sqlx::{PgConnection, Row, postgres::PgRow};
use std::collections::BTreeMap;

pub(super) async fn plans(
    conn: &mut PgConnection,
) -> AdminStoreResult<Vec<SubscriptionPlanRecord>> {
    sqlx::query("select id,name,description,enabled,is_base,\n daily_limit_usd::text,weekly_limit_usd::text,monthly_limit_usd::text,created_at,updated_at from subscription_plans order by is_base desc,created_at desc,id desc")
    .fetch_all(conn)
    .await
    .map_err(|_| unavailable("load plans"))?
    .iter()
    .map(plan_from_row)
    .collect()
}

pub(super) async fn plan(
    conn: &mut PgConnection,
    id: &str,
) -> AdminStoreResult<SubscriptionPlanRecord> {
    let row = sqlx::query("select id,name,description,enabled,is_base,\n daily_limit_usd::text,weekly_limit_usd::text,monthly_limit_usd::text,created_at,updated_at from subscription_plans where id=$1")
        .bind(id)
        .fetch_optional(conn)
        .await
        .map_err(|_| unavailable("load plan"))?
        .ok_or_else(|| not_found("subscription plan", id))?;
    plan_from_row(&row)
}

fn optional_u64(row: &PgRow, name: &str) -> AdminStoreResult<Option<u64>> {
    row.get::<Option<i64>, _>(name)
        .map(u64::try_from)
        .transpose()
        .map_err(|_| invalid("invalid rate limit"))
}

fn plan_from_row(row: &PgRow) -> AdminStoreResult<SubscriptionPlanRecord> {
    Ok(SubscriptionPlanRecord {
        id: row.get("id"),
        name: row.get("name"),
        description: row.get("description"),
        enabled: row.get("enabled"),
        is_base: row.get("is_base"),
        budget_limits: UserBudgetLimits {
            daily_usd: parse_optional_decimal(row.get("daily_limit_usd"))?,
            weekly_usd: parse_optional_decimal(row.get("weekly_limit_usd"))?,
            monthly_usd: parse_optional_decimal(row.get("monthly_limit_usd"))?,
        },
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

pub(super) fn subscription_from_row(row: &PgRow) -> AdminStoreResult<UserSubscriptionRecord> {
    Ok(UserSubscriptionRecord {
        id: row.get("id"),
        user_id: row.get("user_id"),
        plan_id: row.get("plan_id"),
        plan_name: row.get("plan_name"),
        status: row.get("status"),
        starts_at: row.get("starts_at"),
        expires_at: row.get("expires_at"),
        revoked_at: row.get("revoked_at"),
        downstream_rate_multiplier: parse_decimal(
            row.get::<String, _>("downstream_rate_multiplier").as_str(),
        )?,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

pub(super) async fn subscription(
    conn: &mut PgConnection,
    id: &str,
) -> AdminStoreResult<UserSubscriptionRecord> {
    let row = sqlx::query("select s.id,s.user_id,s.plan_id,p.name as plan_name,s.status,s.starts_at,s.expires_at,\n s.revoked_at,s.downstream_rate_multiplier::text,s.created_at,s.updated_at\n from user_subscriptions s join subscription_plans p on p.id=s.plan_id where s.id=$1")
        .bind(id)
        .fetch_optional(conn)
        .await
        .map_err(|_| unavailable("load subscription"))?
        .ok_or_else(|| not_found("subscription", id))?;
    subscription_from_row(&row)
}

pub(super) async fn unrevoked(
    conn: &mut PgConnection,
    user: &str,
) -> AdminStoreResult<Option<UserSubscriptionRecord>> {
    let row = sqlx::query("select s.id,s.user_id,s.plan_id,p.name as plan_name,s.status,s.starts_at,s.expires_at,\n s.revoked_at,s.downstream_rate_multiplier::text,s.created_at,s.updated_at\n from user_subscriptions s join subscription_plans p on p.id=s.plan_id where s.user_id=$1 and s.status='active' order by s.created_at desc,s.id desc limit 1")
        .bind(user).fetch_optional(conn).await.map_err(|_| unavailable("load editable subscription"))?;
    row.as_ref().map(subscription_from_row).transpose()
}

async fn group_map(
    conn: &mut PgConnection,
    users: &[String],
) -> AdminStoreResult<BTreeMap<String, Vec<UserGroupRecord>>> {
    let rows = sqlx::query("select ug.user_id,g.id,g.name,g.enabled,ug.assigned_at from user_account_groups ug
        join account_groups g on g.id=ug.account_group_id where ug.user_id=any($1) order by ug.user_id,g.name,g.id")
        .bind(users).fetch_all(conn).await.map_err(|_| unavailable("load user groups"))?;
    let mut groups: BTreeMap<String, Vec<UserGroupRecord>> = BTreeMap::new();
    for row in rows {
        groups
            .entry(row.get("user_id"))
            .or_default()
            .push(UserGroupRecord {
                group_id: AccountGroupId::new(row.get::<String, _>("id"))
                    .map_err(|_| invalid("invalid group id"))?,
                name: row.get("name"),
                enabled: row.get("enabled"),
                assigned_at: row.get("assigned_at"),
            });
    }
    Ok(groups)
}

pub(super) async fn revision(conn: &mut PgConnection) -> AdminStoreResult<Revision> {
    let value =
        sqlx::query_scalar::<_, i64>("select config_revision from runtime_settings where id=1")
            .fetch_one(conn)
            .await
            .map_err(|_| unavailable("read config revision"))?;
    Revision::new(u64::try_from(value).map_err(|_| invalid("invalid revision"))?)
        .map_err(|_| invalid("invalid revision"))
}

async fn ensure_user(conn: &mut PgConnection, user: &str) -> AdminStoreResult<()> {
    sqlx::query_scalar::<_, String>("select id from users where id=$1")
        .bind(user)
        .fetch_optional(conn)
        .await
        .map_err(|_| unavailable("load user"))?
        .ok_or_else(|| not_found("user", user))?;
    Ok(())
}

pub(super) async fn user_groups(
    conn: &mut PgConnection,
    user: &str,
) -> AdminStoreResult<UserGroups> {
    ensure_user(conn, user).await?;
    let items = group_map(conn, &[user.to_owned()])
        .await?
        .remove(user)
        .unwrap_or_default();
    Ok(UserGroups {
        items,
        revision: revision(conn).await?,
    })
}

/// 未物化的窗口只作读取投影；首次补额由准入 owner 的共用函数物化。
pub(super) struct Window {
    pub kind: BudgetWindowKind,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub used: Decimal,
}

pub(super) async fn windows_at(
    conn: &mut PgConnection,
    users: &[String],
    now: DateTime<Utc>,
) -> AdminStoreResult<BTreeMap<String, Vec<Window>>> {
    let rows = sqlx::query(
        "select u.id,
         case when w.daily_end>$2::timestamptz then w.daily_start else d.day end as daily_start,
         case when w.daily_end>$2::timestamptz then w.daily_end else d.day+interval '24 hours' end as daily_end,
         (case when w.daily_end>$2::timestamptz then w.daily_used_usd else 0 end)::text as daily_used,
         case when w.weekly_end>$2::timestamptz then w.weekly_start else d.week end as weekly_start,
         case when w.weekly_end>$2::timestamptz then w.weekly_end else d.week+interval '168 hours' end as weekly_end,
         (case when w.weekly_end>$2::timestamptz then w.weekly_used_usd else 0 end)::text as weekly_used,
         case when w.monthly_end>$2::timestamptz then w.monthly_start else d.month end as monthly_start,
         case when w.monthly_end>$2::timestamptz then w.monthly_end else d.month_end end as monthly_end,
         (case when w.monthly_end>$2::timestamptz then w.monthly_used_usd else 0 end)::text as monthly_used
         from users u left join user_budget_windows w on w.user_id=u.id
         cross join (select
           date_trunc('day',$2::timestamptz at time zone 'Asia/Shanghai') at time zone 'Asia/Shanghai' as day,
           date_trunc('week',$2::timestamptz at time zone 'Asia/Shanghai') at time zone 'Asia/Shanghai' as week,
           date_trunc('month',$2::timestamptz at time zone 'Asia/Shanghai') at time zone 'Asia/Shanghai' as month,
           (date_trunc('month',$2::timestamptz at time zone 'Asia/Shanghai')+interval '1 month') at time zone 'Asia/Shanghai' as month_end) d
         where u.id=any($1)")
        .bind(users).bind(now).fetch_all(conn).await.map_err(|_| unavailable("load user windows"))?;
    let mut result = BTreeMap::new();
    for row in rows {
        let mut windows = Vec::new();
        for kind in [
            BudgetWindowKind::Daily,
            BudgetWindowKind::Weekly,
            BudgetWindowKind::Monthly,
        ] {
            let name = kind.as_str();
            windows.push(Window {
                kind,
                start: row.get(format!("{name}_start").as_str()),
                end: row.get(format!("{name}_end").as_str()),
                used: parse_decimal(
                    row.get::<String, _>(format!("{name}_used").as_str())
                        .as_str(),
                )?,
            });
        }
        result.insert(row.get("id"), windows);
    }
    Ok(result)
}

pub(super) fn credit_from_row(row: &PgRow) -> AdminStoreResult<UserBudgetCreditRecord> {
    Ok(UserBudgetCreditRecord {
        id: row.get("id"),
        user_id: row.get("user_id"),
        plan_id: row.get("plan_id"),
        subscription_id: row.get("subscription_id"),
        window: BudgetWindowKind::parse(row.get("window_kind"))
            .ok_or_else(|| invalid("invalid credit window"))?,
        window_start: row.get("window_start"),
        window_end: row.get("window_end"),
        amount: parse_decimal(row.get::<String, _>("amount_usd").as_str())?,
        reason: row.get("reason"),
        created_at: row.get("created_at"),
    })
}

pub(super) async fn credit_by_key(
    conn: &mut PgConnection,
    user: &str,
    key: &str,
) -> AdminStoreResult<Option<UserBudgetCreditRecord>> {
    let row = sqlx::query("select id,user_id,plan_id,subscription_id,window_kind,window_start,window_end,\n amount_usd::text,reason,created_at from user_budget_credits where user_id=$1 and idempotency_key=$2")
    .bind(user)
    .bind(key)
    .fetch_optional(conn)
    .await
    .map_err(|_| unavailable("load credit replay"))?;
    row.as_ref().map(credit_from_row).transpose()
}

pub(super) async fn credit_by_id(
    conn: &mut PgConnection,
    id: &str,
) -> AdminStoreResult<UserBudgetCreditRecord> {
    let row = sqlx::query("select id,user_id,plan_id,subscription_id,window_kind,window_start,window_end,\n amount_usd::text,reason,created_at from user_budget_credits where id=$1")
        .bind(id)
        .fetch_one(conn)
        .await
        .map_err(|_| unavailable("load credit"))?;
    credit_from_row(&row)
}

/// 每页固定数量的集合查询，避免管理表格逐用户请求账单。
pub(super) async fn summaries(
    conn: &mut PgConnection,
    users: &[String],
) -> AdminStoreResult<BTreeMap<String, UserBillingSummary>> {
    let now = sqlx::query_scalar::<_, DateTime<Utc>>("select now()")
        .fetch_one(&mut *conn)
        .await
        .map_err(|_| unavailable("read billing time"))?;
    summaries_at(conn, users, now).await
}

pub(super) async fn summaries_at(
    conn: &mut PgConnection,
    users: &[String],
    now: DateTime<Utc>,
) -> AdminStoreResult<BTreeMap<String, UserBillingSummary>> {
    let user_rows = sqlx::query("select id,max_concurrency_override,requests_per_minute_override from users where id=any($1)")
        .bind(users)
        .fetch_all(&mut *conn)
        .await
        .map_err(|_| unavailable("load billing users"))?;
    let plans = plans(conn)
        .await?
        .into_iter()
        .map(|p| (p.id.clone(), p))
        .collect::<BTreeMap<_, _>>();
    let base = plans
        .values()
        .find(|p| p.is_base && p.enabled)
        .ok_or_else(|| unavailable("base plan missing"))?;
    let sub_rows = sqlx::query("select s.id,s.user_id,s.plan_id,p.name as plan_name,s.status,s.starts_at,s.expires_at,\n s.revoked_at,s.downstream_rate_multiplier::text,s.created_at,s.updated_at\n from user_subscriptions s join subscription_plans p on p.id=s.plan_id where s.id in (\n        select id from (select id,status,row_number() over(partition by user_id order by created_at desc,id desc) as n\n        from user_subscriptions where user_id=any($1)) recent where n<=20 or status='active')\n        order by s.created_at desc,s.id desc")
        .bind(users).fetch_all(&mut *conn).await.map_err(|_| unavailable("load billing subscriptions"))?;
    let mut subscriptions: BTreeMap<String, Vec<UserSubscriptionRecord>> = BTreeMap::new();
    for row in sub_rows {
        let sub = subscription_from_row(&row)?;
        subscriptions
            .entry(
                sub.user_id
                    .clone()
                    .ok_or_else(|| unavailable("subscription user missing"))?,
            )
            .or_default()
            .push(sub);
    }
    let mut groups = group_map(conn, users).await?;
    let mut windows = windows_at(conn, users, now).await?;
    let credits = sqlx::query("select user_id,plan_id,subscription_id,window_kind,window_start,window_end,sum(amount_usd)::text as amount
        from user_budget_credits where user_id=any($1) and window_start<=$2 and window_end>$2
        group by user_id,plan_id,subscription_id,window_kind,window_start,window_end")
        .bind(users).bind(now).fetch_all(&mut *conn).await.map_err(|_| unavailable("load active credits"))?;
    let mut summaries = BTreeMap::new();
    for row in user_rows {
        let user_id: String = row.get("id");
        let account_max_concurrency = optional_u64(&row, "max_concurrency_override")?;
        let account_requests_per_minute = optional_u64(&row, "requests_per_minute_override")?;
        let mut history = subscriptions.remove(&user_id).unwrap_or_default();
        let subscription = history
            .iter()
            .find(|s| {
                s.effective_status(now) == "active"
                    && plans.get(&s.plan_id).is_some_and(|p| p.enabled)
            })
            .cloned();
        let plan = subscription
            .as_ref()
            .and_then(|s| plans.get(&s.plan_id))
            .unwrap_or(base)
            .clone();
        history.truncate(20);
        let limits = plan.budget_limits;
        let sub_id = subscription.as_ref().map(|s| s.id.as_str());
        let user_windows = windows
            .remove(&user_id)
            .ok_or_else(|| unavailable("user windows missing"))?;
        let mut budgets = Vec::new();
        for window in user_windows {
            let limit = match window.kind {
                BudgetWindowKind::Daily => limits.daily_usd,
                BudgetWindowKind::Weekly => limits.weekly_usd,
                BudgetWindowKind::Monthly => limits.monthly_usd,
            };
            let credit = credits
                .iter()
                .find(|r| {
                    r.get::<String, _>("user_id") == user_id
                        && r.get::<String, _>("plan_id") == plan.id
                        && r.get::<Option<String>, _>("subscription_id").as_deref() == sub_id
                        && r.get::<String, _>("window_kind") == window.kind.as_str()
                        && r.get::<DateTime<Utc>, _>("window_start") == window.start
                        && r.get::<DateTime<Utc>, _>("window_end") == window.end
                })
                .map(|r| parse_decimal(r.get::<String, _>("amount").as_str()))
                .transpose()?
                .unwrap_or(Decimal::ZERO);
            let effective_limit = limit
                .map(|v| {
                    v.checked_add(credit)
                        .ok_or_else(|| invalid("effective budget overflow"))
                })
                .transpose()?;
            budgets.push(UserBudgetWindowStatus {
                limit,
                used: window.used,
                credit,
                effective_limit,
                resets_at: window.end,
            });
        }
        let mut budgets = budgets.into_iter();
        let budget = UserBudgetStatus {
            daily: budgets
                .next()
                .ok_or_else(|| unavailable("daily window missing"))?,
            weekly: budgets
                .next()
                .ok_or_else(|| unavailable("weekly window missing"))?,
            monthly: budgets
                .next()
                .ok_or_else(|| unavailable("monthly window missing"))?,
        };
        summaries.insert(
            user_id.clone(),
            UserBillingSummary {
                effective_max_concurrency: account_max_concurrency,
                effective_requests_per_minute: account_requests_per_minute,
                subscription,
                plan,
                subscription_history: history,
                budget,
                groups: groups.remove(&user_id).unwrap_or_default(),
            },
        );
    }
    Ok(summaries)
}

fn offset(query: ControlPageQuery) -> AdminStoreResult<i64> {
    if query.page == 0 || !(1..=100).contains(&query.page_size) {
        return Err(invalid("invalid pagination"));
    }
    Ok(i64::from(query.page - 1) * i64::from(query.page_size))
}
fn count(value: i64) -> AdminStoreResult<u64> {
    u64::try_from(value).map_err(|_| invalid("invalid count"))
}

pub(super) async fn history(
    pool: &PgPool,
    user: &str,
    q: ControlPageQuery,
) -> AdminStoreResult<ControlPage<UserSubscriptionRecord>> {
    let skip = offset(q)?;
    let mut tx = begin_read(pool).await?;
    ensure_user(&mut tx, user).await?;
    let total =
        sqlx::query_scalar::<_, i64>("select count(*) from user_subscriptions where user_id=$1")
            .bind(user)
            .fetch_one(&mut *tx)
            .await
            .map_err(|_| unavailable("count subscriptions"))?;
    let rows = sqlx::query("select s.id,s.user_id,s.plan_id,p.name as plan_name,s.status,s.starts_at,s.expires_at,\n s.revoked_at,s.downstream_rate_multiplier::text,s.created_at,s.updated_at\n from user_subscriptions s join subscription_plans p on p.id=s.plan_id where s.user_id=$1 order by s.created_at desc,s.id desc limit $2 offset $3")
    .bind(user)
    .bind(i64::from(q.page_size))
    .bind(skip)
    .fetch_all(&mut *tx)
    .await
    .map_err(|_| unavailable("load history"))?;
    Ok(ControlPage {
        items: rows
            .iter()
            .map(subscription_from_row)
            .collect::<AdminStoreResult<_>>()?,
        page: q.page,
        page_size: q.page_size,
        total: count(total)?,
    })
}

pub(super) async fn credits(
    pool: &PgPool,
    user: &str,
    window: Option<BudgetWindowKind>,
    q: ControlPageQuery,
) -> AdminStoreResult<ControlPage<UserBudgetCreditRecord>> {
    let skip = offset(q)?;
    let mut tx = begin_read(pool).await?;
    ensure_user(&mut tx, user).await?;
    let window = window.map(BudgetWindowKind::as_str);
    let total=sqlx::query_scalar::<_,i64>("select count(*) from user_budget_credits where user_id=$1 and ($2::text is null or window_kind=$2)")
        .bind(user).bind(window).fetch_one(&mut *tx).await.map_err(|_|unavailable("count credits"))?;
    let rows=sqlx::query("select id,user_id,plan_id,subscription_id,window_kind,window_start,window_end,\n amount_usd::text,reason,created_at from user_budget_credits where user_id=$1 and ($2::text is null or window_kind=$2) order by created_at desc,id desc limit $3 offset $4")
        .bind(user).bind(window).bind(i64::from(q.page_size)).bind(skip).fetch_all(&mut *tx).await.map_err(|_|unavailable("load credits"))?;
    Ok(ControlPage {
        items: rows
            .iter()
            .map(credit_from_row)
            .collect::<AdminStoreResult<_>>()?,
        page: q.page,
        page_size: q.page_size,
        total: count(total)?,
    })
}

pub(super) async fn management(
    pool: &PgPool,
    q: UserManagementQuery,
) -> AdminStoreResult<ControlPage<UserManagementRecord>> {
    let skip = offset(q.pagination)?;
    let mut tx = begin_read(pool).await?;
    let role = q.role.map(|r| r.as_str());
    let group = q.group_id.as_ref().map(|g| g.as_str());
    let total = sqlx::query_scalar::<_, i64>("select count(*) from users u\n where ($1::text is null or strpos(lower(u.username),lower($1))>0)\n and ($2::text is null or u.role=$2) and ($3::boolean is null or u.enabled=$3)\n and ($4::text is null or $4=coalesce(\n    (select s.plan_id from user_subscriptions s join subscription_plans p on p.id=s.plan_id and p.enabled\n     where s.user_id=u.id and s.status='active' and s.starts_at<=now() and s.expires_at>now()\n     order by s.created_at desc,s.id desc limit 1),\n    (select id from subscription_plans where is_base and enabled)))\n and ($5::text is null or exists(select 1 from user_account_groups ug where ug.user_id=u.id and ug.account_group_id=$5))")
        .bind(&q.query)
        .bind(role)
        .bind(q.enabled)
        .bind(&q.plan_id)
        .bind(group)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| unavailable("count managed users"))?;
    let rows=sqlx::query("select u.id,u.username,u.role,u.enabled,u.updated_at from users u\n where ($1::text is null or strpos(lower(u.username),lower($1))>0)\n and ($2::text is null or u.role=$2) and ($3::boolean is null or u.enabled=$3)\n and ($4::text is null or $4=coalesce(\n    (select s.plan_id from user_subscriptions s join subscription_plans p on p.id=s.plan_id and p.enabled\n     where s.user_id=u.id and s.status='active' and s.starts_at<=now() and s.expires_at>now()\n     order by s.created_at desc,s.id desc limit 1),\n    (select id from subscription_plans where is_base and enabled)))\n and ($5::text is null or exists(select 1 from user_account_groups ug where ug.user_id=u.id and ug.account_group_id=$5)) order by u.created_at desc,u.id desc limit $6 offset $7")
        .bind(&q.query).bind(role).bind(q.enabled).bind(&q.plan_id).bind(group).bind(i64::from(q.pagination.page_size)).bind(skip)
        .fetch_all(&mut *tx).await.map_err(|_|unavailable("load managed users"))?;
    let ids = rows
        .iter()
        .map(|r| r.get::<String, _>("id"))
        .collect::<Vec<_>>();
    let mut billing = summaries(&mut tx, &ids).await?;
    let mut items = Vec::new();
    for row in rows {
        let id: String = row.get("id");
        items.push(UserManagementRecord {
            billing: billing
                .remove(&id)
                .ok_or_else(|| unavailable("billing summary missing"))?,
            user_id: id,
            username: row.get("username"),
            role: UserRole::parse(row.get("role")).ok_or_else(|| invalid("invalid user role"))?,
            enabled: row.get("enabled"),
            updated_at: row.get("updated_at"),
        });
    }
    Ok(ControlPage {
        items,
        page: q.pagination.page,
        page_size: q.pagination.page_size,
        total: count(total)?,
    })
}
