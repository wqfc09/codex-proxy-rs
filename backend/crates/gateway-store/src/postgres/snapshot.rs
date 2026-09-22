//! 终态配置表的一致性 `RuntimeSnapshot` 输入读取。

use std::collections::BTreeMap;

use async_trait::async_trait;
use gateway_core::account::ProviderAccountId;
use gateway_core::metering::Decimal;
use gateway_core::policy::{ClientBillingPolicy, UserBudgetLimits, UserId, UserRateLimits};
use gateway_core::routing::{
    AccountGroupId, ConfigRevision,
    snapshot::{
        SnapshotAccountGroupFacts, SnapshotAccountGroupMemberFacts, SnapshotClientPolicyFacts,
        SnapshotFacts, SnapshotProviderAccountFacts, SnapshotSettingsFacts, SnapshotStoreError,
        SnapshotStorePort,
    },
};
use sqlx::{PgPool, Postgres, Transaction};

use crate::{Revision, StoreError, StoreResult, postgres_unavailable};

use super::ClientApiKeySnapshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotRuntimeSettings {
    pub pricing: gateway_core::metering::PricingOverrides,
    pub request_profiles:
        BTreeMap<gateway_core::routing::ProviderKind, gateway_core::account::OpaqueProviderData>,
    pub request_location_enabled: bool,
    pub request_location: gateway_core::account::RequestLocation,
    pub refresh_margin_seconds: u64,
    pub refresh_concurrency: u32,
    pub max_concurrent_per_account: u32,
    pub request_interval_ms: u64,
    pub max_waiting_per_key: u32,
    pub max_waiting_per_account: u32,
    pub concurrency_wait_timeout_seconds: u32,
    pub responses_max_decompressed_body_bytes: u64,
    pub rotation_strategy: String,
    pub model_mappings: BTreeMap<String, String>,
    pub min_codex_desktop_version: Option<String>,
    pub min_codex_cli_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSnapshotData {
    pub config_revision: Revision,
    pub observed_current_revision: Revision,
    pub settings: SnapshotRuntimeSettings,
    pub client_api_keys: Vec<ClientApiKeySnapshot>,
    pub account_groups: Vec<SnapshotAccountGroupData>,
    pub provider_accounts: Vec<SnapshotProviderAccountData>,
    pub group_memberships: Vec<SnapshotGroupMembershipData>,
    pub user_runtime: Vec<SnapshotUserRuntimeData>,
    pub client_key_owners: BTreeMap<String, SnapshotClientKeyOwner>,
    pub client_key_metadata: BTreeMap<String, SnapshotClientKeyMetadata>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotClientKeyOwner {
    Ownerless,
    Owned(String),
    Suppressed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotClientKeyMetadata {
    pub username_snapshot: Option<String>,
    pub client_api_key_name_snapshot: Option<String>,
}

/// 0017 用户事实单独读取，避免改变旧 `ClientApiKeySnapshot` 合同。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotUserRuntimeData {
    pub key_id: String,
    pub user_id: String,
    pub group_ids: Vec<String>,
    pub max_concurrency: Option<i64>,
    pub requests_per_minute: Option<i64>,
    pub plan_id: Option<String>,
    pub subscription_id: Option<String>,
    pub daily_limit_usd: Option<String>,
    pub weekly_limit_usd: Option<String>,
    pub monthly_limit_usd: Option<String>,
    pub downstream_rate_multiplier: Option<String>,
    pub starts_at: Option<chrono::DateTime<chrono::Utc>>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub base_plan_id: Option<String>,
    pub base_daily_limit_usd: Option<String>,
    pub base_weekly_limit_usd: Option<String>,
    pub base_monthly_limit_usd: Option<String>,
    pub successor_plan_id: Option<String>,
    pub successor_subscription_id: Option<String>,
    pub successor_daily_limit_usd: Option<String>,
    pub successor_weekly_limit_usd: Option<String>,
    pub successor_monthly_limit_usd: Option<String>,
    pub successor_downstream_rate_multiplier: Option<String>,
    pub successor_starts_at: Option<chrono::DateTime<chrono::Utc>>,
    pub successor_expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotAccountGroupData {
    pub disable_fast: bool,
    pub id: AccountGroupId,
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotProviderAccountData {
    pub id: String,
    pub provider_kind: String,
    pub model_access: gateway_core::account::AccountModelAccess,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotGroupMembershipData {
    pub group_id: AccountGroupId,
    pub account_id: String,
}

#[async_trait]
pub trait RuntimeSnapshotRepository: Send + Sync {
    async fn load_runtime_snapshot(&self) -> StoreResult<RuntimeSnapshotData>;
    async fn current_config_revision(&self) -> StoreResult<Revision>;
}

#[derive(Clone)]
pub struct PgRuntimeSnapshotRepository {
    pool: PgPool,
}

impl PgRuntimeSnapshotRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl RuntimeSnapshotRepository for PgRuntimeSnapshotRepository {
    async fn load_runtime_snapshot(&self) -> StoreResult<RuntimeSnapshotData> {
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(|_| postgres_unavailable("begin runtime snapshot"))?;
        sqlx::query("set transaction isolation level repeatable read read only")
            .execute(&mut *transaction)
            .await
            .map_err(|_| postgres_unavailable("configure runtime snapshot transaction"))?;

        let (config_revision, settings) = load_settings(&mut transaction).await?;
        let client_api_keys = load_client_keys(&mut transaction).await?;
        let account_groups = load_account_groups(&mut transaction).await?;
        let provider_accounts = load_provider_accounts(&mut transaction).await?;
        let group_memberships = load_group_memberships(&mut transaction).await?;
        let user_runtime = load_user_runtime(&mut transaction).await?;
        let (client_key_owners, client_key_metadata) =
            load_client_key_owners(&mut transaction).await?;
        transaction
            .commit()
            .await
            .map_err(|_| postgres_unavailable("commit runtime snapshot"))?;

        let observed_current_revision =
            RuntimeSnapshotRepository::current_config_revision(self).await?;
        Ok(RuntimeSnapshotData {
            config_revision,
            observed_current_revision,
            settings,
            client_api_keys,
            account_groups,
            provider_accounts,
            group_memberships,
            user_runtime,
            client_key_owners,
            client_key_metadata,
        })
    }

    async fn current_config_revision(&self) -> StoreResult<Revision> {
        let revision = sqlx::query_scalar::<_, i64>(
            "select config_revision from runtime_settings where id = 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| postgres_unavailable("read current config revision"))?
        .ok_or_else(|| StoreError::NotFound {
            entity: "runtime settings",
            id: "1".to_owned(),
        })?;
        revision_from_i64(revision)
    }
}

impl SnapshotStorePort for PgRuntimeSnapshotRepository {
    fn load_snapshot_facts(
        &self,
    ) -> futures::future::BoxFuture<'_, Result<SnapshotFacts, SnapshotStoreError>> {
        Box::pin(async move {
            let data = self
                .load_runtime_snapshot()
                .await
                .map_err(|_| SnapshotStoreError::unavailable())?;
            let config_revision = core_revision(data.config_revision)?;
            let observed_current_revision = core_revision(data.observed_current_revision)?;
            let settings = SnapshotSettingsFacts::new(
                data.settings.max_concurrent_per_account,
                data.settings.request_interval_ms,
                data.settings.rotation_strategy,
                data.settings.model_mappings,
                data.settings.min_codex_desktop_version,
                data.settings.min_codex_cli_version,
            )
            .with_responses_max_decompressed_body_bytes(
                data.settings.responses_max_decompressed_body_bytes,
            )
            .with_request_profiles(data.settings.request_profiles)
            .with_pricing(data.settings.pricing)
            .with_request_location(
                data.settings.request_location,
                data.settings.request_location_enabled,
            )
            .with_concurrency_queues(
                data.settings.max_waiting_per_key,
                data.settings.max_waiting_per_account,
                data.settings.concurrency_wait_timeout_seconds,
            );
            let client_policies = data
                .client_api_keys
                .into_iter()
                .filter_map(|key| {
                    let key_id = key.id.as_str().to_owned();
                    let Some(owner) = data.client_key_owners.get(&key_id) else {
                        tracing::warn!(client_key_id = %key_id, "suppressing key without an ownership fact");
                        return None;
                    };
                    if matches!(owner, SnapshotClientKeyOwner::Suppressed) {
                        return None;
                    }
                    let candidate: Result<_, SnapshotStoreError> = (|| {
                        let runtime = data
                            .user_runtime
                            .iter()
                            .find(|runtime| runtime.key_id == key_id);
                        let owned = matches!(owner, SnapshotClientKeyOwner::Owned(_));
                        if owned && runtime.is_none() {
                            return Err(SnapshotStoreError::unavailable());
                        }
                        let group_ids = if let Some(runtime) = runtime {
                            runtime
                                .group_ids
                                .iter()
                                .cloned()
                                .map(gateway_core::routing::AccountGroupId::new)
                                .collect::<Result<Vec<_>, _>>()
                                .map_err(|_| SnapshotStoreError::unavailable())?
                        } else {
                            key.group_ids.clone()
                        };
                        let metadata = data
                            .client_key_metadata
                            .get(&key_id)
                            .ok_or_else(SnapshotStoreError::unavailable)?;
                        let mut policy = SnapshotClientPolicyFacts::new(
                            key.id,
                            key.plaintext_key,
                            group_ids,
                            key.limits,
                        )
                        .with_request_profiles(key.request_profiles)
                        .with_historical_names(
                            metadata.username_snapshot.clone(),
                            metadata.client_api_key_name_snapshot.clone(),
                        );
                        if let Some(runtime) = runtime {
                            let user_id = UserId::new(runtime.user_id.clone())
                                .map_err(|_| SnapshotStoreError::unavailable())?;
                            let billing = build_billing_policy(runtime)
                                .map_err(|_| SnapshotStoreError::unavailable())?;
                            policy = policy.with_user_runtime(
                                user_id,
                                UserRateLimits {
                                    max_concurrency: runtime
                                        .max_concurrency
                                        .map(to_u64_i64)
                                        .transpose()
                                        .map_err(|_| SnapshotStoreError::unavailable())?,
                                    requests_per_minute: runtime
                                        .requests_per_minute
                                        .map(to_u64_i64)
                                        .transpose()
                                        .map_err(|_| SnapshotStoreError::unavailable())?,
                                },
                                billing,
                                true,
                            );
                        } else if owned {
                            return Err(SnapshotStoreError::unavailable());
                        }
                        Ok(policy)
                    })();
                    match candidate {
                        Ok(policy) => Some(policy),
                        Err(_) => {
                            // 单个账户策略损坏只撤销该 Key，不能阻止其他账户接收新快照。
                            // 数据库和全局设置故障仍返回失败，不回退到宽松策略。
                            tracing::warn!(client_key_id = %key_id, "suppressing key with invalid account policy");
                            None
                        }
                    }
                })
                .collect();
            let account_groups = data
                .account_groups
                .into_iter()
                .map(|group| {
                    SnapshotAccountGroupFacts::new(group.id, group.name, group.enabled)
                        .with_disable_fast(group.disable_fast)
                })
                .collect();
            let provider_accounts = data
                .provider_accounts
                .into_iter()
                .map(|account| {
                    ProviderAccountId::new(account.id)
                        .map(|id| {
                            SnapshotProviderAccountFacts::new(id, account.provider_kind)
                                .with_model_access(account.model_access)
                        })
                        .map_err(|_| SnapshotStoreError::unavailable())
                })
                .collect::<Result<Vec<_>, _>>()?;
            let group_memberships = data
                .group_memberships
                .into_iter()
                .map(|membership| {
                    ProviderAccountId::new(membership.account_id)
                        .map(|account_id| {
                            SnapshotAccountGroupMemberFacts::new(membership.group_id, account_id)
                        })
                        .map_err(|_| SnapshotStoreError::unavailable())
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(SnapshotFacts::new(
                config_revision,
                observed_current_revision,
                settings,
                client_policies,
                account_groups,
                provider_accounts,
                group_memberships,
            ))
        })
    }

    fn current_config_revision(
        &self,
    ) -> futures::future::BoxFuture<'_, Result<ConfigRevision, SnapshotStoreError>> {
        Box::pin(async move {
            RuntimeSnapshotRepository::current_config_revision(self)
                .await
                .map_err(|_| SnapshotStoreError::unavailable())
                .and_then(core_revision)
        })
    }
}

fn to_u64_i64(value: i64) -> StoreResult<u64> {
    u64::try_from(value).map_err(|_| StoreError::InvalidData {
        entity: "user runtime policy",
        message: "limit must be non-negative".to_owned(),
    })
}

fn build_billing_policy(
    runtime: &SnapshotUserRuntimeData,
) -> StoreResult<Option<ClientBillingPolicy>> {
    if runtime.base_plan_id.is_none() {
        return Err(StoreError::InvalidData {
            entity: "subscription plan",
            message: "owned user has no enabled base plan".to_owned(),
        });
    }
    let Some(plan_id) = runtime.plan_id.clone() else {
        return Err(StoreError::InvalidData {
            entity: "subscription plan",
            message: "owned user has no enabled active or base plan".to_owned(),
        });
    };
    let plan_id = gateway_core::policy::SubscriptionId::new(plan_id).map_err(|_| {
        StoreError::InvalidData {
            entity: "subscription plan",
            message: "invalid plan ID".to_owned(),
        }
    })?;
    let subscription_id = runtime
        .subscription_id
        .clone()
        .map(gateway_core::policy::SubscriptionId::new)
        .transpose()
        .map_err(|_| StoreError::InvalidData {
            entity: "user subscription",
            message: "invalid subscription ID".to_owned(),
        })?;
    let parse = |value: &Option<String>| -> StoreResult<Option<Decimal>> {
        value
            .as_deref()
            .map(str::parse)
            .transpose()
            .map_err(|_| StoreError::InvalidData {
                entity: "subscription plan",
                message: "invalid budget amount".to_owned(),
            })
    };
    let multiplier = runtime
        .downstream_rate_multiplier
        .as_deref()
        .unwrap_or("1")
        .parse()
        .map_err(|_| StoreError::InvalidData {
            entity: "user subscription",
            message: "invalid billing multiplier".to_owned(),
        })?;
    let selected = ClientBillingPolicy::for_plan_window(
        plan_id,
        subscription_id,
        UserBudgetLimits {
            daily_usd: parse(&runtime.daily_limit_usd)?,
            weekly_usd: parse(&runtime.weekly_limit_usd)?,
            monthly_usd: parse(&runtime.monthly_limit_usd)?,
        },
        multiplier,
        runtime.starts_at.map(Into::into),
        runtime.expires_at.map(Into::into),
    );
    let mut base_fallback = None;
    let selected = if let Some(base_plan_id) = runtime.base_plan_id.clone() {
        let base_plan_id =
            gateway_core::policy::SubscriptionId::new(base_plan_id).map_err(|_| {
                StoreError::InvalidData {
                    entity: "subscription plan",
                    message: "invalid base plan ID".to_owned(),
                }
            })?;
        let base_multiplier: Decimal = "1".parse().map_err(|_| StoreError::InvalidData {
            entity: "subscription plan",
            message: "invalid base multiplier".to_owned(),
        })?;
        let base = ClientBillingPolicy::for_plan(
            base_plan_id,
            None,
            UserBudgetLimits {
                daily_usd: parse(&runtime.base_daily_limit_usd)?,
                weekly_usd: parse(&runtime.base_weekly_limit_usd)?,
                monthly_usd: parse(&runtime.base_monthly_limit_usd)?,
            },
            base_multiplier,
            None,
        );
        base_fallback = Some(base.clone());
        if selected.subscription_id().is_some() {
            selected.with_fallback(base)
        } else {
            selected
        }
    } else {
        selected
    };
    if let Some(successor_plan_id) = runtime.successor_plan_id.clone() {
        let successor_plan_id = gateway_core::policy::SubscriptionId::new(successor_plan_id)
            .map_err(|_| StoreError::InvalidData {
                entity: "subscription plan",
                message: "invalid successor plan ID".to_owned(),
            })?;
        let successor_subscription_id = runtime
            .successor_subscription_id
            .clone()
            .map(gateway_core::policy::SubscriptionId::new)
            .transpose()
            .map_err(|_| StoreError::InvalidData {
                entity: "user subscription",
                message: "invalid successor subscription ID".to_owned(),
            })?;
        let successor_multiplier: Decimal = runtime
            .successor_downstream_rate_multiplier
            .as_deref()
            .unwrap_or("1")
            .parse()
            .map_err(|_| StoreError::InvalidData {
                entity: "user subscription",
                message: "invalid successor billing multiplier".to_owned(),
            })?;
        let successor = ClientBillingPolicy::for_plan_window(
            successor_plan_id,
            successor_subscription_id,
            UserBudgetLimits {
                daily_usd: parse(&runtime.successor_daily_limit_usd)?,
                weekly_usd: parse(&runtime.successor_weekly_limit_usd)?,
                monthly_usd: parse(&runtime.successor_monthly_limit_usd)?,
            },
            successor_multiplier,
            runtime.successor_starts_at.map(Into::into),
            runtime.successor_expires_at.map(Into::into),
        );
        let successor = if let Some(base) = base_fallback {
            successor.with_fallback(base)
        } else {
            successor
        };
        return Ok(Some(selected.with_successor(successor)));
    }
    Ok(Some(selected))
}

fn core_revision(revision: Revision) -> Result<ConfigRevision, SnapshotStoreError> {
    ConfigRevision::new(revision.get()).map_err(|_| SnapshotStoreError::unavailable())
}

#[derive(sqlx::FromRow)]
struct SnapshotSettingsRow {
    pricing_synced_json: sqlx::types::Json<gateway_core::metering::PricingOverrides>,
    pricing_overrides_json: sqlx::types::Json<gateway_core::metering::PricingOverrides>,
    config_revision: i64,
    refresh_margin_seconds: i64,
    refresh_concurrency: i64,
    max_concurrent_per_account: i64,
    request_interval_ms: i64,
    rotation_strategy: String,
    model_mappings_json: sqlx::types::Json<BTreeMap<String, String>>,
    min_codex_desktop_version: Option<String>,
    min_codex_cli_version: Option<String>,
    max_waiting_per_key: i64,
    max_waiting_per_account: i64,
    concurrency_wait_timeout_seconds: i64,
    request_location_json: sqlx::types::Json<gateway_core::account::RequestLocation>,
    request_location_enabled: bool,
    responses_max_decompressed_body_bytes: i64,
    provider_request_profiles_json:
        sqlx::types::Json<BTreeMap<String, serde_json::Map<String, serde_json::Value>>>,
}

async fn load_settings(
    transaction: &mut Transaction<'_, Postgres>,
) -> StoreResult<(Revision, SnapshotRuntimeSettings)> {
    let row = sqlx::query_as::<_, SnapshotSettingsRow>(
        "select config_revision, refresh_margin_seconds, refresh_concurrency, max_concurrent_per_account, request_interval_ms, rotation_strategy, model_mappings_json, min_codex_desktop_version, min_codex_cli_version, max_waiting_per_key, max_waiting_per_account, concurrency_wait_timeout_seconds, request_location_json, request_location_enabled, responses_max_decompressed_body_bytes, provider_request_profiles_json, pricing_overrides_json, pricing_synced_json from runtime_settings where id = 1",
    )
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|_| postgres_unavailable("load snapshot settings"))?
    .ok_or_else(|| StoreError::NotFound {
        entity: "runtime settings",
        id: "1".to_owned(),
    })?;
    Ok((
        revision_from_i64(row.config_revision)?,
        SnapshotRuntimeSettings {
            pricing: {
                super::pricing::validate_pricing(&row.pricing_synced_json.0)?;
                super::pricing::validate_pricing(&row.pricing_overrides_json.0)?;
                gateway_core::metering::merge_pricing(
                    row.pricing_synced_json.0,
                    &row.pricing_overrides_json.0,
                )
            },
            request_profiles: decode_request_profiles(row.provider_request_profiles_json.0)?,
            responses_max_decompressed_body_bytes: to_u64(
                row.responses_max_decompressed_body_bytes,
            )?,
            request_location_enabled: row.request_location_enabled,
            request_location: row.request_location_json.0,
            refresh_margin_seconds: to_u64(row.refresh_margin_seconds)?,
            refresh_concurrency: to_u32(row.refresh_concurrency)?,
            max_concurrent_per_account: to_u32(row.max_concurrent_per_account)?,
            request_interval_ms: to_u64(row.request_interval_ms)?,
            rotation_strategy: row.rotation_strategy,
            model_mappings: row.model_mappings_json.0,
            min_codex_desktop_version: row.min_codex_desktop_version,
            min_codex_cli_version: row.min_codex_cli_version,
            max_waiting_per_key: to_u32(row.max_waiting_per_key)?,
            max_waiting_per_account: to_u32(row.max_waiting_per_account)?,
            concurrency_wait_timeout_seconds: to_u32(row.concurrency_wait_timeout_seconds)?,
        },
    ))
}

async fn load_client_keys(
    transaction: &mut Transaction<'_, Postgres>,
) -> StoreResult<Vec<ClientApiKeySnapshot>> {
    let rows = sqlx::query_as::<
        _,
        (
            String,
            String,
            Vec<String>,
            i64,
            i64,
            sqlx::types::Json<serde_json::Value>,
        ),
    >(
        "select k.id, k.key,
                coalesce(array_agg(kg.account_group_id order by kg.account_group_id)
                  filter (where kg.account_group_id is not null), '{}') as group_ids,
                k.max_concurrency, k.requests_per_minute,
                case when k.owner_user_id is null then k.provider_request_profiles_json
                     else coalesce(u.provider_request_profiles_json, '{}'::jsonb)
                          || k.provider_request_profiles_json end
         from client_api_keys k
         left join users u on u.id = k.owner_user_id
         left join client_api_key_groups kg on kg.client_api_key_id = k.id
         where k.enabled and (k.owner_user_id is null or u.enabled)
         group by k.id, u.id
         order by k.id",
    )
    .fetch_all(&mut **transaction)
    .await
    .map_err(|_| postgres_unavailable("load snapshot client policies"))?;
    Ok(rows.into_iter()
        .filter_map(|row| {
            let key_id = row.0.clone();
            let candidate = (|| -> StoreResult<ClientApiKeySnapshot> {
                let mut key = ClientApiKeySnapshot::from_persisted(row.0, row.1, row.2, row.3, row.4)?;
                let profiles = serde_json::from_value(row.5.0).map_err(|_| StoreError::InvalidData {
                    entity: "client profile", message: "invalid profile object".to_owned(),
                })?;
                key.request_profiles = decode_request_profiles(profiles)?;
                Ok(key)
            })();
            match candidate {
                Ok(key) => Some(key),
                Err(_) => {
                    tracing::warn!(client_key_id = %key_id, "suppressing invalid client key snapshot row");
                    None
                }
            }
        })
        .collect())
}

async fn load_user_runtime(
    transaction: &mut Transaction<'_, Postgres>,
) -> StoreResult<Vec<SnapshotUserRuntimeData>> {
    let rows = sqlx::query_as::<_, SnapshotUserRuntimeRow>(
        "select k.id as key_id, u.id as user_id,
                coalesce((select array_agg(ug.account_group_id order by ug.account_group_id)
                          from user_account_groups ug where ug.user_id = u.id), '{}') as group_ids,
                u.max_concurrency_override, u.requests_per_minute_override,
                case when active.plan_id is not null then active.plan_id else base.plan_id end as plan_id,
                active.subscription_id,
                case when active.plan_id is not null then active.daily_limit_usd else base.daily_limit_usd end as daily_limit_usd,
                case when active.plan_id is not null then active.weekly_limit_usd else base.weekly_limit_usd end as weekly_limit_usd,
                case when active.plan_id is not null then active.monthly_limit_usd else base.monthly_limit_usd end as monthly_limit_usd,
                case when active.plan_id is not null then active.downstream_rate_multiplier else '1'::text end as downstream_rate_multiplier,
                active.starts_at, active.expires_at,
                base.plan_id as base_plan_id, base.daily_limit_usd as base_daily_limit_usd,
                base.weekly_limit_usd as base_weekly_limit_usd,
                base.monthly_limit_usd as base_monthly_limit_usd,
                upcoming.plan_id as successor_plan_id, upcoming.subscription_id as successor_subscription_id,
                upcoming.daily_limit_usd as successor_daily_limit_usd,
                upcoming.weekly_limit_usd as successor_weekly_limit_usd,
                upcoming.monthly_limit_usd as successor_monthly_limit_usd,
                upcoming.downstream_rate_multiplier as successor_downstream_rate_multiplier,
                upcoming.starts_at as successor_starts_at, upcoming.expires_at as successor_expires_at
         from client_api_keys k
         join users u on u.id = k.owner_user_id and u.enabled
         left join lateral (
           select s.id as subscription_id, s.plan_id, s.starts_at, s.expires_at,
                  p.daily_limit_usd::text as daily_limit_usd,
                  p.weekly_limit_usd::text as weekly_limit_usd,
                  p.monthly_limit_usd::text as monthly_limit_usd,
                  s.downstream_rate_multiplier::text as downstream_rate_multiplier
           from user_subscriptions s
           join subscription_plans p on p.id = s.plan_id and p.enabled
           where s.user_id = u.id and s.status = 'active'
             and s.starts_at <= now() and s.expires_at > now()
           order by s.created_at desc, s.id desc limit 1
         ) active on true
         left join lateral (
           select p.id as plan_id, p.daily_limit_usd::text as daily_limit_usd,
                  p.weekly_limit_usd::text as weekly_limit_usd,
                  p.monthly_limit_usd::text as monthly_limit_usd
           from subscription_plans p where p.is_base and p.enabled
           order by p.created_at desc, p.id desc limit 1
         ) base on true
         left join lateral (
           select s.id as subscription_id, s.plan_id, s.starts_at, s.expires_at,
                  p.daily_limit_usd::text as daily_limit_usd,
                  p.weekly_limit_usd::text as weekly_limit_usd,
                  p.monthly_limit_usd::text as monthly_limit_usd,
                  s.downstream_rate_multiplier::text as downstream_rate_multiplier
           from user_subscriptions s
           join subscription_plans p on p.id = s.plan_id and p.enabled
           where s.user_id = u.id and s.status = 'active' and s.starts_at > now()
           order by s.starts_at asc, s.created_at desc, s.id desc limit 1
         ) upcoming on true
         where k.enabled and k.owner_user_id is not null
         order by k.id",
    )
    .fetch_all(&mut **transaction)
    .await
    .map_err(|_| postgres_unavailable("load snapshot user runtime policies"))?;
    rows.into_iter()
        .map(|row| {
            Ok(SnapshotUserRuntimeData {
                key_id: row.key_id,
                user_id: row.user_id,
                group_ids: row.group_ids,
                max_concurrency: row.max_concurrency_override,
                requests_per_minute: row.requests_per_minute_override,
                plan_id: row.plan_id,
                subscription_id: row.subscription_id,
                daily_limit_usd: row.daily_limit_usd,
                weekly_limit_usd: row.weekly_limit_usd,
                monthly_limit_usd: row.monthly_limit_usd,
                downstream_rate_multiplier: row.downstream_rate_multiplier,
                starts_at: row.starts_at,
                expires_at: row.expires_at,
                base_plan_id: row.base_plan_id,
                base_daily_limit_usd: row.base_daily_limit_usd,
                base_weekly_limit_usd: row.base_weekly_limit_usd,
                base_monthly_limit_usd: row.base_monthly_limit_usd,
                successor_plan_id: row.successor_plan_id,
                successor_subscription_id: row.successor_subscription_id,
                successor_daily_limit_usd: row.successor_daily_limit_usd,
                successor_weekly_limit_usd: row.successor_weekly_limit_usd,
                successor_monthly_limit_usd: row.successor_monthly_limit_usd,
                successor_downstream_rate_multiplier: row.successor_downstream_rate_multiplier,
                successor_starts_at: row.successor_starts_at,
                successor_expires_at: row.successor_expires_at,
            })
        })
        .collect()
}

async fn load_client_key_owners(
    transaction: &mut Transaction<'_, Postgres>,
) -> StoreResult<(
    BTreeMap<String, SnapshotClientKeyOwner>,
    BTreeMap<String, SnapshotClientKeyMetadata>,
)> {
    let rows = sqlx::query_as::<_, (String, Option<String>, bool, String, Option<String>)>(
        "select k.id, k.owner_user_id, coalesce(u.enabled, false), k.name, u.username
         from client_api_keys k
         left join users u on u.id = k.owner_user_id
         where k.enabled
         order by k.id",
    )
    .fetch_all(&mut **transaction)
    .await
    .map_err(|_| postgres_unavailable("load snapshot client key owners"))?;
    let mut owners = BTreeMap::new();
    let mut metadata = BTreeMap::new();
    for (key_id, owner_user_id, owner_enabled, key_name, username) in rows {
        let owner = match owner_user_id {
            None => SnapshotClientKeyOwner::Ownerless,
            Some(user_id) if owner_enabled => SnapshotClientKeyOwner::Owned(user_id),
            Some(_) => SnapshotClientKeyOwner::Suppressed,
        };
        if owners.insert(key_id.clone(), owner).is_some() {
            return Err(StoreError::InvalidData {
                entity: "client API key owner",
                message: "duplicate client API key".to_owned(),
            });
        }
        if metadata
            .insert(
                key_id,
                SnapshotClientKeyMetadata {
                    username_snapshot: username,
                    client_api_key_name_snapshot: Some(key_name),
                },
            )
            .is_some()
        {
            return Err(StoreError::InvalidData {
                entity: "client API key metadata",
                message: "duplicate client API key".to_owned(),
            });
        }
    }
    Ok((owners, metadata))
}

#[derive(sqlx::FromRow)]
struct SnapshotUserRuntimeRow {
    key_id: String,
    user_id: String,
    group_ids: Vec<String>,
    max_concurrency_override: Option<i64>,
    requests_per_minute_override: Option<i64>,
    plan_id: Option<String>,
    subscription_id: Option<String>,
    daily_limit_usd: Option<String>,
    weekly_limit_usd: Option<String>,
    monthly_limit_usd: Option<String>,
    downstream_rate_multiplier: Option<String>,
    starts_at: Option<chrono::DateTime<chrono::Utc>>,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    base_plan_id: Option<String>,
    base_daily_limit_usd: Option<String>,
    base_weekly_limit_usd: Option<String>,
    base_monthly_limit_usd: Option<String>,
    successor_plan_id: Option<String>,
    successor_subscription_id: Option<String>,
    successor_daily_limit_usd: Option<String>,
    successor_weekly_limit_usd: Option<String>,
    successor_monthly_limit_usd: Option<String>,
    successor_downstream_rate_multiplier: Option<String>,
    successor_starts_at: Option<chrono::DateTime<chrono::Utc>>,
    successor_expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

async fn load_account_groups(
    transaction: &mut Transaction<'_, Postgres>,
) -> StoreResult<Vec<SnapshotAccountGroupData>> {
    let rows = sqlx::query_as::<_, (String, String, bool, bool)>(
        "select id, name, enabled, disable_fast from account_groups order by id",
    )
    .fetch_all(&mut **transaction)
    .await
    .map_err(|_| postgres_unavailable("load snapshot account groups"))?;
    rows.into_iter()
        .map(|(id, name, enabled, disable_fast)| {
            Ok(SnapshotAccountGroupData {
                disable_fast,
                id: AccountGroupId::new(id).map_err(|_| invalid("invalid account group id"))?,
                name,
                enabled,
            })
        })
        .collect()
}

async fn load_provider_accounts(
    transaction: &mut Transaction<'_, Postgres>,
) -> StoreResult<Vec<SnapshotProviderAccountData>> {
    sqlx::query_as::<
        _,
        (
            String,
            String,
            sqlx::types::Json<gateway_core::account::AccountModelAccess>,
        ),
    >("select id, provider_kind, model_access_json from provider_accounts order by id")
    .fetch_all(&mut **transaction)
    .await
    .map_err(|_| postgres_unavailable("load snapshot provider accounts"))
    .map(|rows| {
        rows.into_iter()
            .map(
                |(id, provider_kind, model_access)| SnapshotProviderAccountData {
                    id,
                    provider_kind,
                    model_access: model_access.0,
                },
            )
            .collect()
    })
}

async fn load_group_memberships(
    transaction: &mut Transaction<'_, Postgres>,
) -> StoreResult<Vec<SnapshotGroupMembershipData>> {
    let rows = sqlx::query_as::<_, (String, String)>(
        "select account_group_id, provider_account_id
         from account_group_accounts order by account_group_id, provider_account_id",
    )
    .fetch_all(&mut **transaction)
    .await
    .map_err(|_| postgres_unavailable("load snapshot group memberships"))?;
    rows.into_iter()
        .map(|(group_id, account_id)| {
            Ok(SnapshotGroupMembershipData {
                group_id: AccountGroupId::new(group_id)
                    .map_err(|_| invalid("invalid membership group id"))?,
                account_id,
            })
        })
        .collect()
}

fn revision_from_i64(value: i64) -> StoreResult<Revision> {
    Revision::new(to_u64(value)?)
}

fn to_u64(value: i64) -> StoreResult<u64> {
    u64::try_from(value).map_err(|_| invalid("numeric snapshot field is negative"))
}

fn to_u32(value: i64) -> StoreResult<u32> {
    u32::try_from(value).map_err(|_| invalid("numeric snapshot field is outside u32"))
}

fn invalid(message: &str) -> StoreError {
    StoreError::InvalidData {
        entity: "runtime snapshot",
        message: message.to_owned(),
    }
}

fn decode_request_profiles(
    profiles: BTreeMap<String, serde_json::Map<String, serde_json::Value>>,
) -> StoreResult<
    BTreeMap<gateway_core::routing::ProviderKind, gateway_core::account::OpaqueProviderData>,
> {
    profiles
        .into_iter()
        .map(|(kind, document)| {
            Ok((
                gateway_core::routing::ProviderKind::new(kind)
                    .map_err(|_| invalid("invalid request profile provider"))?,
                gateway_core::account::OpaqueProviderData::new(document),
            ))
        })
        .collect()
}
