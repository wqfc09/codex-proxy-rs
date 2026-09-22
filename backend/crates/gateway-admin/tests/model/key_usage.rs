use chrono::{Duration, Utc};
use gateway_admin::model::{
    key_usage::KeyUsageBudget,
    subscription_billing::{UserBudgetStatus, UserBudgetWindowStatus},
};
use gateway_core::{
    engine::budget::{ClientBudgetLimits, ClientBudgetStatus},
    metering::Decimal,
};

fn amount(value: &str) -> Decimal {
    value.parse().expect("decimal")
}

fn user_window(limit: Option<&str>, used: &str, resets_in_days: i64) -> UserBudgetWindowStatus {
    UserBudgetWindowStatus {
        limit: limit.map(amount),
        used: amount(used),
        credit: Decimal::ZERO,
        effective_limit: limit.map(amount),
        resets_at: Utc::now() + Duration::days(resets_in_days),
    }
}

#[test]
fn effective_available_uses_the_smallest_remaining_across_key_and_plan_windows() {
    let now = Utc::now();
    let key = ClientBudgetStatus {
        limits: ClientBudgetLimits {
            daily_usd: amount("100"),
            weekly_usd: amount("100"),
        },
        daily_used_usd: amount("10"),
        weekly_used_usd: amount("20"),
        daily_resets_at: Some((now + Duration::days(1)).into()),
        weekly_resets_at: Some((now + Duration::days(7)).into()),
    };
    let user = UserBudgetStatus {
        daily: user_window(Some("50"), "10", 1),
        weekly: user_window(Some("60"), "20", 6),
        monthly: user_window(Some("12"), "10", 20),
    };

    let budget = KeyUsageBudget::resolve(key, Some(&user));
    assert_eq!(budget.available.remaining_usd, Some(amount("2")));
    assert_eq!(budget.available.resets_at, Some(user.monthly.resets_at));
}

#[test]
fn effective_available_ignores_unlimited_windows() {
    let key = ClientBudgetStatus {
        limits: ClientBudgetLimits {
            daily_usd: Decimal::ZERO,
            weekly_usd: amount("5"),
        },
        weekly_used_usd: amount("1.5"),
        ..Default::default()
    };
    let user = UserBudgetStatus {
        daily: user_window(None, "9", 1),
        weekly: user_window(None, "9", 7),
        monthly: user_window(None, "9", 30),
    };

    let budget = KeyUsageBudget::resolve(key, Some(&user));
    assert_eq!(budget.available.remaining_usd, Some(amount("3.5")));
}

#[test]
fn effective_available_is_unlimited_only_when_every_window_is_unlimited() {
    let key = ClientBudgetStatus::default();
    let user = UserBudgetStatus {
        daily: user_window(None, "1", 1),
        weekly: user_window(None, "2", 7),
        monthly: user_window(None, "3", 30),
    };

    let budget = KeyUsageBudget::resolve(key, Some(&user));
    assert_eq!(budget.available.remaining_usd, None);
    assert_eq!(budget.available.resets_at, None);
}
