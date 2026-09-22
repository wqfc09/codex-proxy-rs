//! 下游 Client API Key 的准入策略。
//!
//! Client API Key 冻结账号分组权限；模型名称不参与权限判断。

mod client_version;

pub use client_version::{
    ClientVersionRejection, CodexClientKind, CodexClientMinVersions, CodexClientVersion,
};

use std::{fmt, sync::Arc, time::SystemTime};

use crate::validation::{IdentifierError, PolicyError, validate_text};
use crate::{account::scope::FrozenAccountScope, decimal::Decimal};

/// `client_api_keys.id` 的核心值对象。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClientApiKeyId(String);

impl ClientApiKeyId {
    /// 校验并创建 Key ID。
    ///
    /// # Errors
    ///
    /// ID 为空、过长或包含控制字符时返回错误。
    pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        validate_text(&value, 128, false, None)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ClientApiKeyId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// RuntimeSnapshot 中用于同步认证的明文 Client API Key。
///
/// 数据库按产品约束明文保存；该值对象只负责阻止 `Debug`/日志意外输出。
#[derive(Clone, PartialEq, Eq)]
pub struct PlaintextClientApiKey(String);

impl PlaintextClientApiKey {
    /// 校验并创建明文 Key。
    ///
    /// # Errors
    ///
    /// Key 为空或无法作为 HTTP Bearer 值发送时返回错误。
    pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        Self::validate(&value)?;
        Ok(Self(value))
    }

    /// 迁入的 Key 不限定前缀或长度；保持原值，仅校验 HTTP 可传输的非空可见 ASCII。
    ///
    /// # Errors
    ///
    /// Key 为空或包含空白、控制字符、非 ASCII 字符时返回错误。
    pub fn validate(value: &str) -> Result<(), IdentifierError> {
        if value.is_empty() {
            return Err(IdentifierError::Empty);
        }
        if !value.bytes().all(|byte| byte.is_ascii_graphic()) {
            return Err(IdentifierError::InvalidFormat);
        }
        Ok(())
    }

    /// 仅借给同步认证器做常量时间比较。
    #[must_use]
    pub fn expose_for_auth(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for PlaintextClientApiKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PlaintextClientApiKey(<redacted>)")
    }
}

/// 零表示对应维度不额外限制。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RateLimits {
    pub max_concurrency: u64,
    pub requests_per_minute: u64,
}

impl RateLimits {
    #[must_use]
    pub const fn unlimited() -> Self {
        Self {
            max_concurrency: 0,
            requests_per_minute: 0,
        }
    }
}

/// 用户账户级并发与请求频率限制；`None` 表示不限，`Some(0)` 表示拒绝。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UserRateLimits {
    pub max_concurrency: Option<u64>,
    pub requests_per_minute: Option<u64>,
}
impl UserRateLimits {
    #[must_use]
    pub const fn unlimited() -> Self {
        Self {
            max_concurrency: None,
            requests_per_minute: None,
        }
    }
}

/// 用户账户级金额额度；`None` 表示不限，`Some(0)` 表示零额度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UserBudgetLimits {
    pub daily_usd: Option<Decimal>,
    pub weekly_usd: Option<Decimal>,
    pub monthly_usd: Option<Decimal>,
}
impl UserBudgetLimits {
    #[must_use]
    pub fn is_limited(self) -> bool {
        self.daily_usd.is_some() || self.weekly_usd.is_some() || self.monthly_usd.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UserId(String);
impl UserId {
    pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        validate_text(&value, 128, false, None)?;
        Ok(Self(value))
    }
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SubscriptionId(String);
impl SubscriptionId {
    pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        validate_text(&value, 128, false, None)?;
        Ok(Self(value))
    }
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl fmt::Display for SubscriptionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientBillingPolicy {
    plan_id: SubscriptionId,
    subscription_id: Option<SubscriptionId>,
    budget_limits: UserBudgetLimits,
    downstream_rate_multiplier: Decimal,
    starts_at: Option<SystemTime>,
    expires_at: Option<SystemTime>,
    fallback: Option<Box<Self>>,
    successor: Option<Box<Self>>,
}
impl ClientBillingPolicy {
    #[must_use]
    pub fn for_plan_window(
        plan_id: SubscriptionId,
        subscription_id: Option<SubscriptionId>,
        budget_limits: UserBudgetLimits,
        downstream_rate_multiplier: Decimal,
        starts_at: Option<SystemTime>,
        expires_at: Option<SystemTime>,
    ) -> Self {
        Self {
            plan_id,
            subscription_id,
            budget_limits,
            downstream_rate_multiplier,
            starts_at,
            expires_at,
            fallback: None,
            successor: None,
        }
    }
    #[must_use]
    pub fn for_plan(
        plan_id: SubscriptionId,
        subscription_id: Option<SubscriptionId>,
        budget_limits: UserBudgetLimits,
        downstream_rate_multiplier: Decimal,
        expires_at: Option<SystemTime>,
    ) -> Self {
        Self::for_plan_window(
            plan_id,
            subscription_id,
            budget_limits,
            downstream_rate_multiplier,
            None,
            expires_at,
        )
    }
    #[must_use]
    pub fn is_active_at(&self, now: SystemTime) -> bool {
        self.starts_at.is_none_or(|v| v <= now) && self.expires_at.is_none_or(|v| v > now)
    }

    #[must_use]
    pub fn with_fallback(mut self, fallback: Self) -> Self {
        self.fallback = Some(Box::new(fallback));
        self
    }

    #[must_use]
    pub fn with_successor(mut self, successor: Self) -> Self {
        self.successor = Some(Box::new(successor));
        self
    }

    /// 快照可能跨过订阅的自然到期时间；按请求开始时刻选择冻结套餐。
    #[must_use]
    pub fn effective_at(&self, now: SystemTime) -> Option<Self> {
        if self.is_active_at(now) {
            if let Some(successor) = self.successor.as_deref()
                && successor
                    .starts_at
                    .is_some_and(|starts_at| starts_at <= now)
            {
                successor.effective_at(now)
            } else {
                Some(self.clone())
            }
        } else {
            self.fallback
                .as_deref()
                .and_then(|fallback| fallback.effective_at(now))
        }
    }
    #[must_use]
    pub const fn plan_id(&self) -> &SubscriptionId {
        &self.plan_id
    }
    #[must_use]
    pub const fn subscription_id(&self) -> Option<&SubscriptionId> {
        self.subscription_id.as_ref()
    }
    #[must_use]
    pub const fn budget_limits(&self) -> UserBudgetLimits {
        self.budget_limits
    }
    #[must_use]
    pub const fn downstream_rate_multiplier(&self) -> Decimal {
        self.downstream_rate_multiplier
    }
    #[must_use]
    pub const fn starts_at(&self) -> Option<SystemTime> {
        self.starts_at
    }
    #[must_use]
    pub const fn expires_at(&self) -> Option<SystemTime> {
        self.expires_at
    }
}

/// 从 `client_api_keys` 冻结的公开准入事实。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientPolicy {
    key_id: ClientApiKeyId,
    plaintext_key: PlaintextClientApiKey,
    account_scope: Arc<FrozenAccountScope>,
    enabled: bool,
    limits: RateLimits,
    user_id: Option<UserId>,
    user_rate_limits: UserRateLimits,
    billing_policy: Option<ClientBillingPolicy>,
    username_snapshot: Option<String>,
    client_api_key_name_snapshot: Option<String>,
}

impl ClientPolicy {
    #[must_use]
    pub const fn new(
        key_id: ClientApiKeyId,
        plaintext_key: PlaintextClientApiKey,
        account_scope: Arc<FrozenAccountScope>,
        enabled: bool,
        limits: RateLimits,
    ) -> Self {
        Self {
            key_id,
            plaintext_key,
            account_scope,
            enabled,
            limits,
            user_id: None,
            user_rate_limits: UserRateLimits::unlimited(),
            billing_policy: None,
            username_snapshot: None,
            client_api_key_name_snapshot: None,
        }
    }

    /// 绑定 0017 用户运行时事实；ownerless Key 保留旧的 Key 级语义。
    #[must_use]
    pub fn with_user_runtime(
        mut self,
        user_id: UserId,
        user_rate_limits: UserRateLimits,
        billing_policy: Option<ClientBillingPolicy>,
    ) -> Self {
        self.user_id = Some(user_id);
        self.user_rate_limits = user_rate_limits;
        self.billing_policy = billing_policy;
        self
    }

    /// 绑定请求历史所需的不可变名称快照；ownerless Key 的用户名保持为空。
    #[must_use]
    pub fn with_historical_names(
        mut self,
        username_snapshot: Option<String>,
        client_api_key_name_snapshot: Option<String>,
    ) -> Self {
        self.username_snapshot = username_snapshot;
        self.client_api_key_name_snapshot = client_api_key_name_snapshot;
        self
    }

    #[must_use]
    pub const fn key_id(&self) -> &ClientApiKeyId {
        &self.key_id
    }

    #[must_use]
    pub const fn plaintext_key(&self) -> &PlaintextClientApiKey {
        &self.plaintext_key
    }

    #[must_use]
    pub const fn account_scope(&self) -> &Arc<FrozenAccountScope> {
        &self.account_scope
    }

    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub const fn limits(&self) -> RateLimits {
        self.limits
    }

    #[must_use]
    pub const fn user_id(&self) -> Option<&UserId> {
        self.user_id.as_ref()
    }

    #[must_use]
    pub const fn user_rate_limits(&self) -> UserRateLimits {
        self.user_rate_limits
    }

    #[must_use]
    pub const fn billing_policy(&self) -> Option<&ClientBillingPolicy> {
        self.billing_policy.as_ref()
    }

    #[must_use]
    pub fn username_snapshot(&self) -> Option<&str> {
        self.username_snapshot.as_deref()
    }

    #[must_use]
    pub fn client_api_key_name_snapshot(&self) -> Option<&str> {
        self.client_api_key_name_snapshot.as_deref()
    }

    /// 禁用的 Key 不接受新请求。
    ///
    /// # Errors
    ///
    /// Key 已禁用时返回稳定拒绝原因。
    pub fn authorize(&self) -> Result<(), PolicyError> {
        if self.enabled {
            Ok(())
        } else {
            Err(PolicyError::Denied {
                reason: "client API key is disabled",
            })
        }
    }
}
