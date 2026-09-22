//! 单页 Key 用量与 Bearer Key 额度查询合同；身份范围不由调用方提供。

use chrono::{DateTime, Utc};
use gateway_core::{engine::budget::ClientBudgetStatus, metering::Decimal};

use super::{
    PageSize,
    client_keys::ClientKeyRecord,
    observability::{
        HealthTimeline, OpsErrorPage, RequestMetricPoint, TimeRange, UsageOverview, UsagePage,
    },
    subscription_billing::{UserBudgetStatus, UserBudgetWindowStatus},
};

#[derive(Debug, Clone)]
pub struct KeyUsageQuery {
    pub range: TimeRange,
    pub model: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum KeyUsageRecordKind {
    Success,
    Error,
}

#[derive(Debug, Clone)]
pub struct KeyUsageRecordsQuery {
    pub usage: KeyUsageQuery,
    pub kind: KeyUsageRecordKind,
    pub current_page: u32,
    pub page_size: PageSize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyUsageAvailableBudget {
    pub remaining_usd: Option<Decimal>,
    pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyUsageBudget {
    pub key: ClientBudgetStatus,
    pub available: KeyUsageAvailableBudget,
}

impl KeyUsageBudget {
    #[must_use]
    pub fn resolve(key: ClientBudgetStatus, user: Option<&UserBudgetStatus>) -> Self {
        let mut tightest = None;
        push_candidate(
            &mut tightest,
            key_window(
                key.limits.daily_usd,
                key.daily_used_usd,
                key.daily_resets_at.map(DateTime::<Utc>::from),
            ),
        );
        push_candidate(
            &mut tightest,
            key_window(
                key.limits.weekly_usd,
                key.weekly_used_usd,
                key.weekly_resets_at.map(DateTime::<Utc>::from),
            ),
        );
        if let Some(user) = user {
            for window in [&user.daily, &user.weekly, &user.monthly] {
                push_candidate(&mut tightest, user_window(window));
            }
        }

        let available = tightest.map_or(
            KeyUsageAvailableBudget {
                remaining_usd: None,
                resets_at: None,
            },
            |candidate| KeyUsageAvailableBudget {
                remaining_usd: Some(candidate.remaining_usd),
                resets_at: candidate.resets_at,
            },
        );
        Self { key, available }
    }
}

#[derive(Debug, Clone)]
struct BudgetCandidate {
    remaining_usd: Decimal,
    resets_at: Option<DateTime<Utc>>,
}

fn key_window(
    limit: Decimal,
    used: Decimal,
    resets_at: Option<DateTime<Utc>>,
) -> Option<BudgetCandidate> {
    (limit != Decimal::ZERO).then(|| BudgetCandidate {
        remaining_usd: limit.saturating_sub(used),
        resets_at,
    })
}

fn user_window(window: &UserBudgetWindowStatus) -> Option<BudgetCandidate> {
    window.effective_limit.map(|limit| BudgetCandidate {
        remaining_usd: limit.saturating_sub(window.used),
        resets_at: Some(window.resets_at),
    })
}

fn push_candidate(tightest: &mut Option<BudgetCandidate>, candidate: Option<BudgetCandidate>) {
    let Some(candidate) = candidate else {
        return;
    };
    let Some(current) = tightest else {
        *tightest = Some(candidate);
        return;
    };
    if candidate.remaining_usd < current.remaining_usd {
        *current = candidate;
        return;
    }
    if candidate.remaining_usd == current.remaining_usd {
        current.resets_at = match (&current.resets_at, &candidate.resets_at) {
            (Some(current), Some(candidate)) => Some((*current).max(*candidate)),
            _ => None,
        };
    }
}

pub struct KeyUsageOverview {
    pub key: ClientKeyRecord,
    pub overview: UsageOverview,
    pub trend: Vec<RequestMetricPoint>,
    pub health_timeline: HealthTimeline,
}

pub enum KeyUsageRecords {
    Success(UsagePage),
    Error(OpsErrorPage),
}
