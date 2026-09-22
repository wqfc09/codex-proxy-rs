//! Redis 丢失后从 `model_requests` 恢复客户端准入热状态。

use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use gateway_core::{
    engine::{
        ModelRequestId,
        admission::{
            ClientAdmissionError, ClientAdmissionRecovery as CoreAdmissionRecovery,
            ClientAdmissionRecoveryPort, RecentAdmissionFact, RunningAdmissionFact,
            UserAdmissionRecovery as CoreUserAdmissionRecovery,
        },
    },
    policy::{ClientApiKeyId, UserId},
};
use sqlx::PgPool;

use crate::{StoreResult, postgres_unavailable};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientAdmissionRecentRequest {
    pub model_request_id: String,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientAdmissionRunningRequest {
    pub model_request_id: String,
    pub deadline_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientAdmissionUserRecovery {
    pub user_id: String,
    pub recent_requests: Vec<ClientAdmissionRecentRequest>,
    pub running_requests: Vec<ClientAdmissionRunningRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientAdmissionRecovery {
    pub client_api_key_ref: String,
    pub recent_requests: Vec<ClientAdmissionRecentRequest>,
    pub running_requests: Vec<ClientAdmissionRunningRequest>,
    pub user: Option<ClientAdmissionUserRecovery>,
}

#[async_trait]
pub trait ClientAdmissionRecoveryRepository: Send + Sync {
    async fn load_client_admission_recovery(
        &self,
        window_started_at: DateTime<Utc>,
    ) -> StoreResult<Vec<ClientAdmissionRecovery>>;
}

#[derive(Clone)]
pub struct PgClientAdmissionRecoveryRepository {
    pool: PgPool,
}

impl PgClientAdmissionRecoveryRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ClientAdmissionRecoveryRepository for PgClientAdmissionRecoveryRepository {
    async fn load_client_admission_recovery(
        &self,
        window_started_at: DateTime<Utc>,
    ) -> StoreResult<Vec<ClientAdmissionRecovery>> {
        let rows = sqlx::query_as::<
            _,
            (
                String,
                String,
                DateTime<Utc>,
                DateTime<Utc>,
                String,
                Option<String>,
            ),
        >(
            "select mr.client_api_key_ref, mr.id, mr.started_at, mr.deadline_at, mr.outcome,
                    u.id
             from model_requests mr
             left join users u on u.id = mr.user_id and u.enabled
             where mr.started_at >= $1 or mr.outcome = 'running'
             order by mr.client_api_key_ref, mr.started_at, mr.id",
        )
        .bind(window_started_at)
        .fetch_all(&self.pool)
        .await
        .map_err(|_| postgres_unavailable("load client admission recovery"))?;
        let mut recoveries = BTreeMap::<String, ClientAdmissionRecovery>::new();
        for (client_api_key_ref, model_request_id, started_at, deadline_at, outcome, user_id) in
            rows
        {
            let recovery = recoveries
                .entry(client_api_key_ref.clone())
                .or_insert_with(|| ClientAdmissionRecovery {
                    client_api_key_ref,
                    recent_requests: Vec::new(),
                    running_requests: Vec::new(),
                    user: None,
                });
            if let Some(user_id) = user_id {
                let user = recovery
                    .user
                    .get_or_insert_with(|| ClientAdmissionUserRecovery {
                        user_id: user_id.clone(),
                        recent_requests: Vec::new(),
                        running_requests: Vec::new(),
                    });
                // User 归属取请求冻结的 user_id；即使当前 Key 已删除，也不能丢失
                // 已准入请求的共享 User 窗口事实或用当前所有者重新解释历史。
                if user.user_id == user_id {
                    if started_at >= window_started_at {
                        user.recent_requests.push(ClientAdmissionRecentRequest {
                            model_request_id: model_request_id.clone(),
                            started_at,
                        });
                    }
                    if outcome == "running" {
                        user.running_requests.push(ClientAdmissionRunningRequest {
                            model_request_id: model_request_id.clone(),
                            deadline_at,
                        });
                    }
                }
            }
            if started_at >= window_started_at {
                recovery.recent_requests.push(ClientAdmissionRecentRequest {
                    model_request_id: model_request_id.clone(),
                    started_at,
                });
            }
            if outcome == "running" {
                recovery
                    .running_requests
                    .push(ClientAdmissionRunningRequest {
                        model_request_id,
                        deadline_at,
                    });
            }
        }
        Ok(recoveries.into_values().collect())
    }
}

impl ClientAdmissionRecoveryPort for PgClientAdmissionRecoveryRepository {
    fn load_recovery(
        &self,
        since: std::time::SystemTime,
    ) -> futures::future::BoxFuture<'_, Result<Vec<CoreAdmissionRecovery>, ClientAdmissionError>>
    {
        Box::pin(async move {
            self.load_client_admission_recovery(DateTime::<Utc>::from(since))
                .await
                .map_err(|_| ClientAdmissionError)?
                .into_iter()
                .map(|recovery| {
                    let client_api_key_id = ClientApiKeyId::new(recovery.client_api_key_ref)
                        .map_err(|_| ClientAdmissionError)?;
                    let recent_requests = recovery
                        .recent_requests
                        .into_iter()
                        .map(|request| {
                            Ok(RecentAdmissionFact {
                                model_request_id: ModelRequestId::new(request.model_request_id)
                                    .map_err(|_| ClientAdmissionError)?,
                                started_at: request.started_at.into(),
                            })
                        })
                        .collect::<Result<Vec<_>, ClientAdmissionError>>()?;
                    let running_requests = recovery
                        .running_requests
                        .into_iter()
                        .map(|request| {
                            Ok(RunningAdmissionFact {
                                model_request_id: ModelRequestId::new(request.model_request_id)
                                    .map_err(|_| ClientAdmissionError)?,
                                expires_at: request.deadline_at.into(),
                            })
                        })
                        .collect::<Result<Vec<_>, ClientAdmissionError>>()?;
                    let user = recovery
                        .user
                        .map(|user| {
                            let user_id =
                                UserId::new(user.user_id).map_err(|_| ClientAdmissionError)?;
                            let recent_requests = user
                                .recent_requests
                                .into_iter()
                                .map(|request| {
                                    Ok(RecentAdmissionFact {
                                        model_request_id: ModelRequestId::new(
                                            request.model_request_id,
                                        )
                                        .map_err(|_| ClientAdmissionError)?,
                                        started_at: request.started_at.into(),
                                    })
                                })
                                .collect::<Result<Vec<_>, ClientAdmissionError>>()?;
                            let running_requests = user
                                .running_requests
                                .into_iter()
                                .map(|request| {
                                    Ok(RunningAdmissionFact {
                                        model_request_id: ModelRequestId::new(
                                            request.model_request_id,
                                        )
                                        .map_err(|_| ClientAdmissionError)?,
                                        expires_at: request.deadline_at.into(),
                                    })
                                })
                                .collect::<Result<Vec<_>, ClientAdmissionError>>()?;
                            Ok(CoreUserAdmissionRecovery {
                                user_id,
                                recent_requests,
                                running_requests,
                            })
                        })
                        .transpose()?;
                    Ok(CoreAdmissionRecovery {
                        client_api_key_id,
                        recent_requests,
                        running_requests,
                        user,
                    })
                })
                .collect()
        })
    }
}
