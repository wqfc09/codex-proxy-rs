//! Client API Key 的 Redis RPM/并发原子准入与热状态恢复。

use std::{collections::HashSet, time::Duration};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use gateway_core::engine::admission::{
    ClientAdmissionDecision as CoreAdmissionDecision, ClientAdmissionError as CoreAdmissionError,
    ClientAdmissionPort, ClientAdmissionRecovery as CoreAdmissionRecovery,
    ClientAdmissionRejection as CoreAdmissionRejection,
    ClientAdmissionRequest as CoreAdmissionRequest,
    ClientAdmissionRestoreResult as CoreAdmissionRestoreResult,
};
use redis::{Script, aio::ConnectionManager};

use crate::{StoreError, StoreResult, redis_unavailable, require_nonempty};

use super::{MAX_REDIS_EXACT_INTEGER, namespace, resource_fingerprint};

const ADMIT_SCRIPT: &str = r#"
local clock = redis.call('TIME')
local now_ms = (tonumber(clock[1]) * 1000) + math.floor(tonumber(clock[2]) / 1000)
local cutoff = now_ms - 60000
local lease_ttl_ms = tonumber(ARGV[2])
if now_ms + lease_ttl_ms > tonumber(ARGV[5]) then return 3 end
local has_user = tonumber(ARGV[7]) == 1
redis.call('ZREMRANGEBYSCORE', KEYS[1], '-inf', now_ms)
redis.call('ZREMRANGEBYSCORE', KEYS[2], '-inf', cutoff)
if has_user then
  redis.call('ZREMRANGEBYSCORE', KEYS[3], '-inf', now_ms)
  redis.call('ZREMRANGEBYSCORE', KEYS[4], '-inf', cutoff)
end

if tonumber(ARGV[4]) > 0 and redis.call('ZCARD', KEYS[2]) >= tonumber(ARGV[4]) then
  return 1
end
if has_user and tonumber(ARGV[9]) > 0 and redis.call('ZCARD', KEYS[4]) >= tonumber(ARGV[9]) then
  return 1
end
if tonumber(ARGV[6]) == 0 then
  return 2
end
if tonumber(ARGV[3]) > 0 and redis.call('ZCARD', KEYS[1]) >= tonumber(ARGV[3]) then
  return 2
end
if has_user and tonumber(ARGV[8]) > 0 and redis.call('ZCARD', KEYS[3]) >= tonumber(ARGV[8]) then
  return 2
end

redis.call('ZADD', KEYS[1], now_ms + lease_ttl_ms, ARGV[1])
redis.call('ZADD', KEYS[2], now_ms, ARGV[1])
if has_user then
  redis.call('ZADD', KEYS[3], now_ms + lease_ttl_ms, ARGV[1])
  redis.call('ZADD', KEYS[4], now_ms, ARGV[1])
end

local function extend_ttl(key, ttl)
  local current = redis.call('PTTL', key)
  if current < ttl then redis.call('PEXPIRE', key, ttl) end
end

local active_tail = redis.call('ZRANGE', KEYS[1], -1, -1, 'WITHSCORES')
local active_ttl = 120000
if #active_tail == 2 then
  active_ttl = math.max(active_ttl, tonumber(active_tail[2]) - now_ms + 60000)
end
extend_ttl(KEYS[1], active_ttl)
extend_ttl(KEYS[2], 120000)
if has_user then
  local user_active_tail = redis.call('ZRANGE', KEYS[3], -1, -1, 'WITHSCORES')
  local user_active_ttl = 120000
  if #user_active_tail == 2 then
    user_active_ttl = math.max(user_active_ttl, tonumber(user_active_tail[2]) - now_ms + 60000)
  end
  extend_ttl(KEYS[3], user_active_ttl)
  extend_ttl(KEYS[4], 120000)
end
return 0
"#;

const RELEASE_SCRIPT: &str = r#"
local removed = redis.call('ZREM', KEYS[1], ARGV[1])
if #KEYS >= 4 then
  redis.call('ZREM', KEYS[3], ARGV[1])
end
return removed
"#;

const RESTORE_SCRIPT: &str = r#"
local clock = redis.call('TIME')
local now_ms = (tonumber(clock[1]) * 1000) + math.floor(tonumber(clock[2]) / 1000)
local cutoff = now_ms - 60000
local recent_count = tonumber(ARGV[1])
local cursor = 2

for _ = 1, recent_count do
  local started_at_ms = tonumber(ARGV[cursor + 1])
  if started_at_ms > now_ms then return {-1, 0, 0} end
  cursor = cursor + 2
end

local running_count = tonumber(ARGV[cursor])
local running_cursor = cursor + 1
local metadata_cursor = running_cursor + (running_count * 2)
local has_user = tonumber(ARGV[metadata_cursor])
metadata_cursor = metadata_cursor + 1
local user_recent_count = 0
local user_recent_cursor = metadata_cursor
local user_running_count = 0
local user_running_cursor = metadata_cursor
if has_user == 1 then
  user_recent_count = tonumber(ARGV[metadata_cursor])
  user_recent_cursor = metadata_cursor + 1
  user_running_cursor = user_recent_cursor + (user_recent_count * 2)
  user_running_count = tonumber(ARGV[user_running_cursor])
  user_running_cursor = user_running_cursor + 1
end

if has_user == 1 then
  for _ = 1, user_recent_count do
    local started_at_ms = tonumber(ARGV[user_recent_cursor + 1])
    if started_at_ms > now_ms then return {-1, 0, 0} end
    user_recent_cursor = user_recent_cursor + 2
  end
end

redis.call('ZREMRANGEBYSCORE', KEYS[1], '-inf', now_ms)
redis.call('ZREMRANGEBYSCORE', KEYS[2], '-inf', cutoff)
if has_user == 1 then
  redis.call('ZREMRANGEBYSCORE', KEYS[3], '-inf', now_ms)
  redis.call('ZREMRANGEBYSCORE', KEYS[4], '-inf', cutoff)
end

local restored_recent = 0
cursor = 2
for _ = 1, recent_count do
  local request_id = ARGV[cursor]
  local started_at_ms = tonumber(ARGV[cursor + 1])
  if started_at_ms > cutoff then
    local inserted = redis.call('ZADD', KEYS[2], 'NX', started_at_ms, request_id)
    if inserted == 1 then
      restored_recent = restored_recent + 1
    end
  end
  cursor = cursor + 2
end

local restored_running = 0
for _ = 1, running_count do
  local request_id = ARGV[running_cursor]
  local expires_at_ms = tonumber(ARGV[running_cursor + 1])
  if expires_at_ms > now_ms then
    restored_running = restored_running
      + redis.call('ZADD', KEYS[1], 'NX', expires_at_ms, request_id)
  end
  running_cursor = running_cursor + 2
end

if has_user == 1 then
  local user_recent_cursor = user_recent_cursor - (user_recent_count * 2)
  for _ = 1, user_recent_count do
    local request_id = ARGV[user_recent_cursor]
    local started_at_ms = tonumber(ARGV[user_recent_cursor + 1])
    if started_at_ms > cutoff then
      redis.call('ZADD', KEYS[4], 'NX', started_at_ms, request_id)
    end
    user_recent_cursor = user_recent_cursor + 2
  end
  for _ = 1, user_running_count do
    local request_id = ARGV[user_running_cursor]
    local expires_at_ms = tonumber(ARGV[user_running_cursor + 1])
    if expires_at_ms > now_ms then
      redis.call('ZADD', KEYS[3], 'NX', expires_at_ms, request_id)
    end
    user_running_cursor = user_running_cursor + 2
  end
end

local function extend_ttl(key, ttl)
  if redis.call('ZCARD', key) == 0 then return end
  local current = redis.call('PTTL', key)
  if current < ttl then redis.call('PEXPIRE', key, ttl) end
end

local active_tail = redis.call('ZRANGE', KEYS[1], -1, -1, 'WITHSCORES')
local active_ttl = 120000
if #active_tail == 2 then
  active_ttl = math.max(active_ttl, tonumber(active_tail[2]) - now_ms + 60000)
end
extend_ttl(KEYS[1], active_ttl)
extend_ttl(KEYS[2], 120000)
if has_user == 1 then
  local user_active_tail = redis.call('ZRANGE', KEYS[3], -1, -1, 'WITHSCORES')
  local user_active_ttl = 120000
  if #user_active_tail == 2 then
    user_active_ttl = math.max(user_active_ttl, tonumber(user_active_tail[2]) - now_ms + 60000)
  end
  extend_ttl(KEYS[3], user_active_ttl)
  extend_ttl(KEYS[4], 120000)
end
return {0, restored_recent, restored_running}
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientAdmissionLimits {
    pub max_concurrency: u64,
    pub requests_per_minute: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientAdmissionRequest {
    pub model_request_id: String,
    pub client_api_key_ref: String,
    pub lease_ttl: Duration,
    pub allow_concurrency_acquire: bool,
    pub limits: ClientAdmissionLimits,
    pub user: Option<UserAdmissionScope>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserAdmissionScope {
    pub user_id: String,
    pub limits: gateway_core::policy::UserRateLimits,
}

impl ClientAdmissionRequest {
    pub fn validate(&self) -> StoreResult<()> {
        require_nonempty(
            "client admission",
            "model_request_id",
            &self.model_request_id,
        )?;
        require_nonempty(
            "client admission",
            "client_api_key_ref",
            &self.client_api_key_ref,
        )?;
        if self.lease_ttl.is_zero() {
            return Err(invalid("lease TTL must be positive"));
        }
        redis_duration_millis(self.lease_ttl)?;
        redis_integer(self.limits.max_concurrency, "maximum concurrency")?;
        redis_integer(self.limits.requests_per_minute, "requests per minute")?;
        if let Some(user) = &self.user {
            require_nonempty("client admission", "user ID", &user.user_id)?;
            redis_integer(
                user.limits.max_concurrency.unwrap_or(0),
                "user maximum concurrency",
            )?;
            redis_integer(
                user.limits.requests_per_minute.unwrap_or(0),
                "user requests per minute",
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientAdmissionRecentRequest {
    pub model_request_id: String,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientAdmissionRunningRequest {
    pub model_request_id: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientAdmissionUserRestore {
    pub user_id: String,
    pub recent_requests: Vec<ClientAdmissionRecentRequest>,
    pub running_requests: Vec<ClientAdmissionRunningRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientAdmissionRestore {
    pub client_api_key_ref: String,
    pub recent_requests: Vec<ClientAdmissionRecentRequest>,
    pub running_requests: Vec<ClientAdmissionRunningRequest>,
    pub user: Option<ClientAdmissionUserRestore>,
}

impl ClientAdmissionRestore {
    pub fn validate(&self) -> StoreResult<()> {
        require_nonempty(
            "client admission",
            "client_api_key_ref",
            &self.client_api_key_ref,
        )?;
        redis_len(self.recent_requests.len(), "recent request count")?;
        redis_len(self.running_requests.len(), "running request count")?;

        let mut recent_ids = HashSet::with_capacity(self.recent_requests.len());
        for request in &self.recent_requests {
            validate_recovery_request_id(&request.model_request_id)?;
            redis_timestamp_millis(request.started_at, "request start time")?;
            if !recent_ids.insert(request.model_request_id.as_str()) {
                return Err(invalid("recent request IDs must be unique"));
            }
        }

        let mut running_ids = HashSet::with_capacity(self.running_requests.len());
        for request in &self.running_requests {
            validate_recovery_request_id(&request.model_request_id)?;
            redis_timestamp_millis(request.expires_at, "request expiry time")?;
            if !running_ids.insert(request.model_request_id.as_str()) {
                return Err(invalid("running request IDs must be unique"));
            }
        }
        if let Some(user) = &self.user {
            require_nonempty("client admission", "user ID", &user.user_id)?;
            redis_len(user.recent_requests.len(), "user recent request count")?;
            redis_len(user.running_requests.len(), "user running request count")?;
            let mut recent_ids = HashSet::with_capacity(user.recent_requests.len());
            for request in &user.recent_requests {
                validate_recovery_request_id(&request.model_request_id)?;
                redis_timestamp_millis(request.started_at, "user request start time")?;
                if !recent_ids.insert(request.model_request_id.as_str()) {
                    return Err(invalid("user recent request IDs must be unique"));
                }
            }
            let mut running_ids = HashSet::with_capacity(user.running_requests.len());
            for request in &user.running_requests {
                validate_recovery_request_id(&request.model_request_id)?;
                redis_timestamp_millis(request.expires_at, "user request expiry time")?;
                if !running_ids.insert(request.model_request_id.as_str()) {
                    return Err(invalid("user running request IDs must be unique"));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientAdmissionRestoreResult {
    pub restored_recent_requests: u64,
    pub restored_running_requests: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientAdmissionRejection {
    RateLimited,
    ConcurrencyLimited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientAdmissionDecision {
    Granted,
    Rejected(ClientAdmissionRejection),
}

#[async_trait]
pub trait ClientAdmissionRepository: Send + Sync {
    async fn admit_client_request(
        &self,
        request: &ClientAdmissionRequest,
    ) -> StoreResult<ClientAdmissionDecision>;
    async fn release_client_request(
        &self,
        client_api_key_ref: &str,
        model_request_id: &str,
    ) -> StoreResult<bool>;
    async fn release_client_request_scoped(
        &self,
        client_api_key_ref: &str,
        user_id: Option<&str>,
        model_request_id: &str,
    ) -> StoreResult<bool>;
    async fn restore_client_admission(
        &self,
        recovery: &ClientAdmissionRestore,
    ) -> StoreResult<ClientAdmissionRestoreResult>;
    async fn clear_client_admission(&self, client_api_key_ref: &str) -> StoreResult<()>;
}

#[derive(Clone)]
pub struct RedisClientAdmissionRepository {
    connection: ConnectionManager,
    namespace: String,
}

impl RedisClientAdmissionRepository {
    pub fn new(connection: ConnectionManager, key_namespace: &str) -> StoreResult<Self> {
        Ok(Self {
            connection,
            namespace: namespace(key_namespace)?,
        })
    }

    fn keys(&self, client_api_key_ref: &str) -> StoreResult<[String; 2]> {
        let fingerprint = resource_fingerprint("client admission", client_api_key_ref)?;
        let tag = format!("{{{fingerprint}}}");
        Ok([
            format!("{}:client:{tag}:active", self.namespace),
            format!("{}:client:{tag}:requests", self.namespace),
        ])
    }

    fn user_keys(&self, user_id: &str) -> StoreResult<[String; 2]> {
        let fingerprint = resource_fingerprint("client admission user", user_id)?;
        let tag = format!("{{{fingerprint}}}");
        Ok([
            format!("{}:user:{tag}:active", self.namespace),
            format!("{}:user:{tag}:requests", self.namespace),
        ])
    }
}

#[async_trait]
impl ClientAdmissionRepository for RedisClientAdmissionRepository {
    async fn admit_client_request(
        &self,
        request: &ClientAdmissionRequest,
    ) -> StoreResult<ClientAdmissionDecision> {
        request.validate()?;
        // 不依赖Core调用者预检查；Some(0)在转换为Lua参数前就拒绝，不写任何桶。
        if let Some(user) = &request.user {
            if user.limits.requests_per_minute == Some(0) {
                return Ok(ClientAdmissionDecision::Rejected(
                    ClientAdmissionRejection::RateLimited,
                ));
            }
            if user.limits.max_concurrency == Some(0) {
                return Ok(ClientAdmissionDecision::Rejected(
                    ClientAdmissionRejection::ConcurrencyLimited,
                ));
            }
        }
        let keys = self.keys(&request.client_api_key_ref)?;
        let lease_ttl_ms = u64::try_from(request.lease_ttl.as_millis())
            .map_err(|_| invalid("lease TTL is too large"))?;
        let mut connection = self.connection.clone();
        let script = Script::new(ADMIT_SCRIPT);
        let mut invocation = script.prepare_invoke();
        invocation.key(&keys[0]).key(&keys[1]);
        if let Some(user) = &request.user {
            let user_keys = self.user_keys(&user.user_id)?;
            invocation.key(&user_keys[0]).key(&user_keys[1]);
        }
        let user = request.user.as_ref();
        let code = invocation
            .arg(&request.model_request_id)
            .arg(lease_ttl_ms)
            .arg(request.limits.max_concurrency)
            .arg(request.limits.requests_per_minute)
            .arg(MAX_REDIS_EXACT_INTEGER)
            .arg(u8::from(request.allow_concurrency_acquire))
            .arg(u8::from(user.is_some()))
            .arg(
                user.and_then(|user| user.limits.max_concurrency)
                    .unwrap_or(0),
            )
            .arg(
                user.and_then(|user| user.limits.requests_per_minute)
                    .unwrap_or(0),
            )
            .invoke_async::<i64>(&mut connection)
            .await
            .map_err(|_| redis_unavailable("admit client request"))?;
        match code {
            0 => Ok(ClientAdmissionDecision::Granted),
            1 => Ok(ClientAdmissionDecision::Rejected(
                ClientAdmissionRejection::RateLimited,
            )),
            2 => Ok(ClientAdmissionDecision::Rejected(
                ClientAdmissionRejection::ConcurrencyLimited,
            )),
            3 => Err(invalid("lease expiry is outside the supported range")),
            _ => Err(invalid("Redis returned an unknown admission decision")),
        }
    }

    async fn release_client_request(
        &self,
        client_api_key_ref: &str,
        model_request_id: &str,
    ) -> StoreResult<bool> {
        self.release_client_request_scoped(client_api_key_ref, None, model_request_id)
            .await
    }

    async fn release_client_request_scoped(
        &self,
        client_api_key_ref: &str,
        user_id: Option<&str>,
        model_request_id: &str,
    ) -> StoreResult<bool> {
        require_nonempty("client admission", "model_request_id", model_request_id)?;
        let keys = self.keys(client_api_key_ref)?;
        let user_keys = user_id.map(|id| self.user_keys(id)).transpose()?;
        let mut connection = self.connection.clone();
        let script = Script::new(RELEASE_SCRIPT);
        let mut invocation = script.prepare_invoke();
        invocation.key(&keys[0]).key(&keys[1]);
        if let Some(user_keys) = user_keys {
            invocation.key(&user_keys[0]).key(&user_keys[1]);
        }
        let removed = invocation
            .arg(model_request_id)
            .invoke_async::<i64>(&mut connection)
            .await
            .map_err(|_| redis_unavailable("release client request"))?;
        Ok(removed == 1)
    }

    async fn restore_client_admission(
        &self,
        recovery: &ClientAdmissionRestore,
    ) -> StoreResult<ClientAdmissionRestoreResult> {
        recovery.validate()?;
        let keys = self.keys(&recovery.client_api_key_ref)?;
        let script = Script::new(RESTORE_SCRIPT);
        let mut invocation = script.prepare_invoke();
        invocation.key(&keys[0]).key(&keys[1]).arg(redis_len(
            recovery.recent_requests.len(),
            "recent request count",
        )?);
        for request in &recovery.recent_requests {
            invocation
                .arg(&request.model_request_id)
                .arg(redis_timestamp_millis(
                    request.started_at,
                    "request start time",
                )?);
        }
        invocation.arg(redis_len(
            recovery.running_requests.len(),
            "running request count",
        )?);
        for request in &recovery.running_requests {
            invocation
                .arg(&request.model_request_id)
                .arg(redis_timestamp_millis(
                    request.expires_at,
                    "request expiry time",
                )?);
        }
        if let Some(user) = &recovery.user {
            let user_keys = self.user_keys(&user.user_id)?;
            invocation.key(&user_keys[0]).key(&user_keys[1]);
            invocation.arg(1_u8).arg(redis_len(
                user.recent_requests.len(),
                "user recent request count",
            )?);
            for request in &user.recent_requests {
                invocation
                    .arg(&request.model_request_id)
                    .arg(redis_timestamp_millis(
                        request.started_at,
                        "user request start time",
                    )?);
            }
            invocation.arg(redis_len(
                user.running_requests.len(),
                "user running request count",
            )?);
            for request in &user.running_requests {
                invocation
                    .arg(&request.model_request_id)
                    .arg(redis_timestamp_millis(
                        request.expires_at,
                        "user request expiry time",
                    )?);
            }
        } else {
            invocation.arg(0_u8);
        }

        let mut connection = self.connection.clone();
        let (code, restored_recent_requests, restored_running_requests) = invocation
            .invoke_async::<(i64, u64, u64)>(&mut connection)
            .await
            .map_err(|_| redis_unavailable("restore client admission"))?;
        if code == -1 {
            return Err(invalid("request start time is after Redis server time"));
        }
        if code != 0 {
            return Err(invalid("Redis returned an unknown recovery decision"));
        }
        Ok(ClientAdmissionRestoreResult {
            restored_recent_requests,
            restored_running_requests,
        })
    }

    async fn clear_client_admission(&self, client_api_key_ref: &str) -> StoreResult<()> {
        let keys = self.keys(client_api_key_ref)?;
        let mut connection = self.connection.clone();
        redis::cmd("DEL")
            .arg(&keys)
            .query_async::<i64>(&mut connection)
            .await
            .map_err(|_| redis_unavailable("clear client admission"))?;
        Ok(())
    }
}

impl ClientAdmissionPort for RedisClientAdmissionRepository {
    fn abandon(
        &self,
        key: &gateway_core::policy::ClientApiKeyId,
        request: &gateway_core::engine::ModelRequestId,
    ) {
        let repository = self.clone();
        let key = key.clone();
        let request = request.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            drop(runtime.spawn(async move {
                if let Err(error) = repository.release(&key, &request).await {
                    tracing::warn!(%error, "已取消准入的租约释放失败，依赖 TTL 收敛");
                }
            }));
        }
    }

    fn abandon_scoped(
        &self,
        key: &gateway_core::policy::ClientApiKeyId,
        user_id: Option<&gateway_core::policy::UserId>,
        request: &gateway_core::engine::ModelRequestId,
    ) {
        let repository = self.clone();
        let key = key.clone();
        let user_id = user_id.cloned();
        let request = request.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            drop(runtime.spawn(async move {
                if let Err(error) = repository
                    .release_client_request_scoped(
                        key.as_str(),
                        user_id.as_ref().map(|id| id.as_str()),
                        request.as_str(),
                    )
                    .await
                {
                    tracing::warn!(%error, "已取消准入的双层租约释放失败，依赖 TTL 收敛");
                }
            }));
        }
    }

    fn admit(
        &self,
        request: CoreAdmissionRequest,
    ) -> futures::future::BoxFuture<'_, Result<CoreAdmissionDecision, CoreAdmissionError>> {
        Box::pin(async move {
            self.admit_client_request(&ClientAdmissionRequest {
                model_request_id: request.model_request_id.as_str().to_owned(),
                client_api_key_ref: request.client_api_key_id.as_str().to_owned(),
                lease_ttl: request.lease_ttl,
                allow_concurrency_acquire: request.allow_concurrency_acquire,
                limits: ClientAdmissionLimits {
                    max_concurrency: request.limits.max_concurrency,
                    requests_per_minute: request.limits.requests_per_minute,
                },
                user: request.user.map(|user| UserAdmissionScope {
                    user_id: user.user_id.as_str().to_owned(),
                    limits: user.limits,
                }),
            })
            .await
            .map(|decision| match decision {
                ClientAdmissionDecision::Granted => CoreAdmissionDecision::Granted,
                ClientAdmissionDecision::Rejected(ClientAdmissionRejection::RateLimited) => {
                    CoreAdmissionDecision::Rejected(CoreAdmissionRejection::RateLimited)
                }
                ClientAdmissionDecision::Rejected(ClientAdmissionRejection::ConcurrencyLimited) => {
                    CoreAdmissionDecision::Rejected(CoreAdmissionRejection::ConcurrencyLimited)
                }
            })
            .map_err(|_| CoreAdmissionError)
        })
    }

    fn release<'a>(
        &'a self,
        client_api_key_id: &'a gateway_core::policy::ClientApiKeyId,
        model_request_id: &'a gateway_core::engine::ModelRequestId,
    ) -> futures::future::BoxFuture<'a, Result<bool, CoreAdmissionError>> {
        Box::pin(async move {
            self.release_client_request(client_api_key_id.as_str(), model_request_id.as_str())
                .await
                .map_err(|_| CoreAdmissionError)
        })
    }

    fn release_scoped<'a>(
        &'a self,
        client_api_key_id: &'a gateway_core::policy::ClientApiKeyId,
        user_id: Option<&'a gateway_core::policy::UserId>,
        model_request_id: &'a gateway_core::engine::ModelRequestId,
    ) -> futures::future::BoxFuture<'a, Result<bool, CoreAdmissionError>> {
        Box::pin(async move {
            self.release_client_request_scoped(
                client_api_key_id.as_str(),
                user_id.map(|id| id.as_str()),
                model_request_id.as_str(),
            )
            .await
            .map_err(|_| CoreAdmissionError)
        })
    }

    fn restore(
        &self,
        recovery: CoreAdmissionRecovery,
    ) -> futures::future::BoxFuture<'_, Result<CoreAdmissionRestoreResult, CoreAdmissionError>>
    {
        Box::pin(async move {
            self.restore_client_admission(&ClientAdmissionRestore {
                client_api_key_ref: recovery.client_api_key_id.as_str().to_owned(),
                recent_requests: recovery
                    .recent_requests
                    .into_iter()
                    .map(|request| ClientAdmissionRecentRequest {
                        model_request_id: request.model_request_id.as_str().to_owned(),
                        started_at: DateTime::<Utc>::from(request.started_at),
                    })
                    .collect(),
                running_requests: recovery
                    .running_requests
                    .into_iter()
                    .map(|request| ClientAdmissionRunningRequest {
                        model_request_id: request.model_request_id.as_str().to_owned(),
                        expires_at: DateTime::<Utc>::from(request.expires_at),
                    })
                    .collect(),
                user: recovery.user.map(|user| ClientAdmissionUserRestore {
                    user_id: user.user_id.as_str().to_owned(),
                    recent_requests: user
                        .recent_requests
                        .into_iter()
                        .map(|request| ClientAdmissionRecentRequest {
                            model_request_id: request.model_request_id.as_str().to_owned(),
                            started_at: DateTime::<Utc>::from(request.started_at),
                        })
                        .collect(),
                    running_requests: user
                        .running_requests
                        .into_iter()
                        .map(|request| ClientAdmissionRunningRequest {
                            model_request_id: request.model_request_id.as_str().to_owned(),
                            expires_at: DateTime::<Utc>::from(request.expires_at),
                        })
                        .collect(),
                }),
            })
            .await
            .map(|restored| CoreAdmissionRestoreResult {
                restored_recent_requests: restored.restored_recent_requests,
                restored_running_requests: restored.restored_running_requests,
            })
            .map_err(|_| CoreAdmissionError)
        })
    }
}

fn invalid(message: &str) -> StoreError {
    StoreError::InvalidData {
        entity: "client admission",
        message: message.to_owned(),
    }
}

fn validate_recovery_request_id(model_request_id: &str) -> StoreResult<()> {
    require_nonempty("client admission", "model_request_id", model_request_id)
}

fn redis_duration_millis(duration: Duration) -> StoreResult<u64> {
    let milliseconds =
        u64::try_from(duration.as_millis()).map_err(|_| invalid("lease TTL is too large"))?;
    redis_integer(milliseconds, "lease TTL")
}

fn redis_timestamp_millis(timestamp: DateTime<Utc>, field: &str) -> StoreResult<u64> {
    let milliseconds = u64::try_from(timestamp.timestamp_millis()).map_err(|_| invalid(field))?;
    redis_integer(milliseconds, field)
}

fn redis_len(length: usize, field: &str) -> StoreResult<u64> {
    let value = u64::try_from(length).map_err(|_| invalid(field))?;
    redis_integer(value, field)
}

fn redis_integer(value: u64, field: &str) -> StoreResult<u64> {
    if value > MAX_REDIS_EXACT_INTEGER {
        return Err(invalid(&format!(
            "{field} is outside Redis' exact integer range"
        )));
    }
    Ok(value)
}
