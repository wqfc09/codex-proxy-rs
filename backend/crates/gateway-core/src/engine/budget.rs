//! 按 Key 持久化 USD 限额与已取得费用，不依赖尽力写入的请求观测。

use std::time::SystemTime;

use futures::future::BoxFuture;

use crate::{
    error::GatewayError,
    metering::Decimal,
    policy::{ClientApiKeyId, ClientBillingPolicy, UserId},
};

use super::ModelRequestId;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClientBudgetLimits {
    pub daily_usd: Decimal,
    pub weekly_usd: Decimal,
}

impl ClientBudgetLimits {
    #[must_use]
    pub fn is_limited(self) -> bool {
        self.daily_usd != Decimal::ZERO || self.weekly_usd != Decimal::ZERO
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientBudgetStatus {
    pub limits: ClientBudgetLimits,
    pub daily_used_usd: Decimal,
    pub weekly_used_usd: Decimal,
    pub daily_resets_at: Option<SystemTime>,
    pub weekly_resets_at: Option<SystemTime>,
}

#[derive(Debug, Clone)]
pub struct ClientBudgetCharge {
    pub key_id: ClientApiKeyId,
    pub request_id: ModelRequestId,
    /// 已取得的 USD 费用，包含重试；缺少费用的尝试按零累计。
    pub amount_usd: Decimal,
    pub completed_at: SystemTime,
}

#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("client budget store is unavailable")]
pub struct ClientBudgetError;

pub trait ClientBudgetPort: Send + Sync {
    /// 原子检查当前限额与已用金额，不创建预扣费或待结算记录。
    fn admit(&self, key_id: ClientApiKeyId) -> BoxFuture<'_, Result<(), GatewayError>>;

    /// 按网关请求 ID 幂等累计已取得费用；写入失败由 Store 重试。
    fn settle(&self, charge: ClientBudgetCharge) -> BoxFuture<'_, Result<(), ClientBudgetError>>;

    /// 用户套餐准入；旧实现默认不可用，ownerless Key 不调用此方法。
    fn admit_user(
        &self,
        _user_id: UserId,
        _policy: ClientBillingPolicy,
    ) -> BoxFuture<'_, Result<(), GatewayError>> {
        Box::pin(async {
            Err(GatewayError::new(
                crate::error::GatewayErrorKind::Internal,
                "user budget store is unavailable",
            ))
        })
    }

    /// 用户套餐结算；未知费用必须由实现跳过账本写入。
    fn settle_user(
        &self,
        _charge: UserBudgetCharge,
    ) -> BoxFuture<'_, Result<(), ClientBudgetError>> {
        Box::pin(async { Err(ClientBudgetError) })
    }
}

#[derive(Debug, Clone)]
pub struct UserBudgetCharge {
    pub user_id: UserId,
    pub key_id: ClientApiKeyId,
    pub request_id: ModelRequestId,
    /// `None` 表示上游没有可信 USD 费用，不产生 zero charge。
    pub base_amount_usd: Option<Decimal>,
    pub policy: ClientBillingPolicy,
    pub completed_at: SystemTime,
}
