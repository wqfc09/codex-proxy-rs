use gateway_admin::{
    model::{MutationActor, MutationContext, subscription_billing::ResetUserBudget},
    ports::store::SubscriptionBillingStore,
};
use gateway_store::postgres::PgSubscriptionBillingStore;

use super::TestDatabase;

#[tokio::test]
async fn user_budget_reset_uses_user_windows_and_preserves_unselected_usage_and_history() {
    let Some(db) = TestDatabase::create("user_budget_reset").await else {
        return;
    };
    sqlx::query(
        "insert into users(id,username,password_hash,role,created_at,updated_at)
        values ('reset_owner','reset-owner','test-only-hash','user',now(),now())",
    )
    .execute(&db.pool)
    .await
    .expect("seed owner without client keys");
    let store = PgSubscriptionBillingStore::new(db.pool.clone());
    let context = MutationContext {
        actor: MutationActor::System,
        request_id: "reset-user-budget".to_owned(),
    };
    // 账户没有任何 Key；初始化自身额度窗口不应尝试建立 Key 账本。
    store
        .reset_user_budget(
            ResetUserBudget {
                user_id: "reset_owner".to_owned(),
                daily: true,
                weekly: false,
                monthly: false,
            },
            &context,
        )
        .await
        .expect("initialize User window independently of Key");
    sqlx::query("update user_budget_windows set daily_used_usd=3,weekly_used_usd=7,monthly_used_usd=11 where user_id='reset_owner'")
        .execute(&db.pool).await.expect("seed usage");
    sqlx::query("insert into user_budget_credits(id,user_id,plan_id,subscription_id,window_kind,window_start,window_end,amount_usd,idempotency_key,reason)
        select 'credit_'||k.kind, w.user_id, p.id, null, k.kind,
          case k.kind when 'daily' then w.daily_start when 'weekly' then w.weekly_start else w.monthly_start end,
          case k.kind when 'daily' then w.daily_end when 'weekly' then w.weekly_end else w.monthly_end end,
          1, 'reset_'||k.kind, 'test selected window'
        from user_budget_windows w cross join subscription_plans p
        cross join (values ('daily'),('weekly'),('monthly')) k(kind)
        where w.user_id='reset_owner' and p.is_base")
        .execute(&db.pool).await.expect("seed window credits");
    sqlx::query("insert into user_charge_events(request_id,user_id,plan_id,base_amount_usd,downstream_rate_multiplier,billed_amount_usd,completed_at)
        select 'reset_history','reset_owner',id,3,1,3,now() from subscription_plans where is_base")
        .execute(&db.pool).await.expect("seed immutable charge history");
    let policy_before: serde_json::Value =
        sqlx::query_scalar("select row_to_json(p) from subscription_plans p where is_base")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    store
        .reset_user_budget(
            ResetUserBudget {
                user_id: "reset_owner".to_owned(),
                daily: true,
                weekly: false,
                monthly: false,
            },
            &context,
        )
        .await
        .expect("reset only daily usage and credit");
    let usage: (String,String,String) = sqlx::query_as("select daily_used_usd::text,weekly_used_usd::text,monthly_used_usd::text from user_budget_windows where user_id='reset_owner'")
        .fetch_one(&db.pool).await.unwrap();
    assert_eq!(
        usage,
        (
            "0.0000000000".to_owned(),
            "7.0000000000".to_owned(),
            "11.0000000000".to_owned()
        )
    );
    let credits: Vec<String> = sqlx::query_scalar("select window_kind from user_budget_credits where user_id='reset_owner' order by window_kind")
        .fetch_all(&db.pool).await.unwrap();
    assert_eq!(credits, vec!["monthly", "weekly"]);
    let events: i64 =
        sqlx::query_scalar("select count(*) from user_charge_events where user_ref='reset_owner'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(events, 1);
    let key_windows: i64 = sqlx::query_scalar("select count(*) from client_key_budget_windows")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(key_windows, 0);
    let policy_after: serde_json::Value =
        sqlx::query_scalar("select row_to_json(p) from subscription_plans p where is_base")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(policy_before, policy_after);
    db.close().await;
}

#[tokio::test]
async fn migration_seeds_one_enabled_base_plan_and_allows_fallback_charge() {
    let Some(db) = TestDatabase::create("base_plan").await else {
        return;
    };
    let base_count: i64 =
        sqlx::query_scalar("select count(*) from subscription_plans where is_base and enabled")
            .fetch_one(&db.pool)
            .await
            .expect("count enabled base plans");
    assert_eq!(base_count, 1);
    let plan_id: String =
        sqlx::query_scalar("select id from subscription_plans where is_base and enabled")
            .fetch_one(&db.pool)
            .await
            .expect("load base plan");

    // 无显式订阅不等于无账户；此处只验证基础套餐账单的存储合同。
    sqlx::query(
        "insert into users (id, username, password_hash, role, created_at, updated_at)
         values ('fallback_user', 'fallback-user', 'test-only-hash', 'user', now(), now())",
    )
    .execute(&db.pool)
    .await
    .expect("create fallback account");
    sqlx::query(
        "insert into user_charge_events
         (request_id, user_id, plan_id, base_amount_usd, downstream_rate_multiplier,
          billed_amount_usd, completed_at)
         values ('fallback_charge', 'fallback_user', $1, 3.5, 1, 3.5, now())",
    )
    .bind(&plan_id)
    .execute(&db.pool)
    .await
    .expect("record charge against fallback plan without subscription");
    let stored: (
        String,
        String,
        Option<String>,
        Option<String>,
        String,
        String,
    ) = sqlx::query_as(
        "select user_ref, plan_id, subscription_id, subscription_ref,
                downstream_rate_multiplier::text, billed_amount_usd::text
         from user_charge_events where request_id = 'fallback_charge'",
    )
    .fetch_one(&db.pool)
    .await
    .expect("read fallback charge");
    assert_eq!(
        stored,
        (
            "fallback_user".to_owned(),
            plan_id,
            None,
            None,
            "1.0000000000".to_owned(),
            "3.5000000000".to_owned(),
        )
    );
    db.close().await;
}

#[tokio::test]
async fn natural_week_projection_credit_and_reset_share_the_same_user_window_without_a_key() {
    use gateway_admin::model::subscription_billing::{AddUserBudgetCredit, BudgetWindowKind};
    let Some(db) = TestDatabase::create("user_natural_week").await else {
        return;
    };
    sqlx::query("insert into users(id,username,password_hash,role,created_at,updated_at) values('week_user','week-user','test-only','user',now(),now())").execute(&db.pool).await.unwrap();
    sqlx::query("update subscription_plans set weekly_limit_usd=10 where is_base")
        .execute(&db.pool)
        .await
        .unwrap();
    let store = PgSubscriptionBillingStore::new(db.pool.clone());
    let expected_end: chrono::DateTime<chrono::Utc> = sqlx::query_scalar("select (date_trunc('week',clock_timestamp() at time zone 'Asia/Shanghai') + interval '7 days') at time zone 'Asia/Shanghai'").fetch_one(&db.pool).await.unwrap();
    let before = store.user_billing_summary("week_user").await.unwrap();
    assert_eq!(
        before.budget.weekly.resets_at, expected_end,
        "projection before first write is the natural week, not seven days after today"
    );
    let context = MutationContext {
        actor: MutationActor::System,
        request_id: "week-credit".to_owned(),
    };
    let (_, credit) = store
        .add_user_budget_credit(
            AddUserBudgetCredit {
                id: "week_credit".to_owned(),
                user_id: "week_user".to_owned(),
                window: BudgetWindowKind::Weekly,
                amount: "2".parse().unwrap(),
                idempotency_key: "weekly-once".to_owned(),
                reason: "weekly boundary test".to_owned(),
            },
            &context,
        )
        .await
        .unwrap();
    assert_eq!(credit.window_end, expected_end);
    assert_eq!(
        credit.window_start,
        expected_end - chrono::Duration::days(7)
    );
    let after = store.user_billing_summary("week_user").await.unwrap();
    assert_eq!(after.budget.weekly.resets_at, expected_end);
    assert_eq!(after.budget.weekly.credit.canonical(), "2");
    assert_eq!(
        after.budget.weekly.effective_limit.unwrap().canonical(),
        "12"
    );
    store
        .reset_user_budget(
            ResetUserBudget {
                user_id: "week_user".to_owned(),
                daily: false,
                weekly: true,
                monthly: false,
            },
            &context,
        )
        .await
        .unwrap();
    let reset = store.user_billing_summary("week_user").await.unwrap();
    assert_eq!(reset.budget.weekly.credit.canonical(), "0");
    assert_eq!(reset.budget.weekly.resets_at, expected_end);
    db.close().await;
}
