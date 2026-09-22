//! 用户 Key 的列表游标与安全响应；不包含管理员 Key 控制面。
use crate::admin::WireValidationError;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use gateway_admin::model::client_keys::{
    ClientKeyBudgetPeriod, ClientKeyCursor, ClientKeyCursorValue as DomainCursorValue,
    ClientKeyListQuery, ClientKeyMutation, ClientKeyPage, ClientKeyPageSize, ClientKeyRecord,
    ClientKeySecret, ClientKeySort as DomainSort, ClientKeySortField as DomainSortField,
    CreatedClientKey, ResetClientKeyBudget, SortDirection,
};
use gateway_core::policy::ClientApiKeyId;
use serde::{Deserialize, Serialize};
use std::fmt;

const MAX_CURSOR_BYTES: usize = 512;
const MAX_SEARCH_BYTES: usize = 256;
const DEFAULT_PAGE_SIZE: u16 = 50;

/// Client Key 列表查询。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListClientKeysQuery {
    cursor: Option<String>,
    limit: Option<u16>,
    search: Option<String>,
    sort_by: Option<String>,
    sort_direction: Option<String>,
}

impl ListClientKeysQuery {
    /// 校验 wire 边界并直接构造管理用例查询。
    pub fn into_command(self) -> Result<ClientKeyListQuery, WireValidationError> {
        if self
            .cursor
            .as_deref()
            .is_some_and(|cursor| cursor.is_empty() || cursor.len() > MAX_CURSOR_BYTES)
        {
            return Err(WireValidationError::new("cursor"));
        }
        if self.limit == Some(0) {
            return Err(WireValidationError::new("limit"));
        }
        let search = self.search.map(|search| search.trim().to_owned());
        if search.as_deref().is_some_and(|search| {
            search.len() > MAX_SEARCH_BYTES || search.chars().any(char::is_control)
        }) {
            return Err(WireValidationError::new("search"));
        }
        let sort = ClientKeySort::parse(
            self.sort_by.as_deref().unwrap_or("createdAt"),
            self.sort_direction.as_deref().unwrap_or("desc"),
        )?;
        let cursor = self
            .cursor
            .as_deref()
            .map(decode_client_key_cursor)
            .transpose()?
            .map(domain_cursor)
            .transpose()?;
        let page_size = ClientKeyPageSize::new(self.limit.unwrap_or(DEFAULT_PAGE_SIZE))
            .map_err(|_| WireValidationError::new("limit"))?;
        Ok(ClientKeyListQuery {
            cursor,
            page_size,
            search: search.filter(|search| !search.is_empty()),
            sort: domain_sort(sort),
        })
    }
}

/// Client Key 数据库排序字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClientKeySortField {
    Name,
    Enabled,
    CreatedAt,
    LastUsedAt,
}

/// Client Key 数据库排序方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClientKeySortDirection {
    Asc,
    Desc,
}

/// 已校验且会写入自描述游标的排序组合。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClientKeySort {
    pub field: ClientKeySortField,
    pub direction: ClientKeySortDirection,
}

impl ClientKeySort {
    fn parse(field: &str, direction: &str) -> Result<Self, WireValidationError> {
        let field = match field {
            "name" => ClientKeySortField::Name,
            "enabled" => ClientKeySortField::Enabled,
            "createdAt" => ClientKeySortField::CreatedAt,
            "lastUsedAt" => ClientKeySortField::LastUsedAt,
            _ => return Err(WireValidationError::new("sortBy")),
        };
        let direction = match direction {
            "asc" => ClientKeySortDirection::Asc,
            "desc" => ClientKeySortDirection::Desc,
            _ => return Err(WireValidationError::new("sortDirection")),
        };
        Ok(Self { field, direction })
    }
}

/// 重置指定周期已用金额的请求。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResetClientKeyBudgetRequest {
    id: String,
    period: BudgetResetPeriod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum BudgetResetPeriod {
    Daily,
    Weekly,
    All,
}

impl ResetClientKeyBudgetRequest {
    pub fn into_command(self) -> Result<ResetClientKeyBudget, WireValidationError> {
        validate_required_text(&self.id, "id")?;
        Ok(ResetClientKeyBudget {
            id: client_key_id(self.id, "clientKeyMutationNotFound")?,
            period: match self.period {
                BudgetResetPeriod::Daily => ClientKeyBudgetPeriod::Daily,
                BudgetResetPeriod::Weekly => ClientKeyBudgetPeriod::Weekly,
                BudgetResetPeriod::All => ClientKeyBudgetPeriod::All,
            },
        })
    }
}

/// 不含完整 Key 的用户安全视图。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientKeyView {
    id: String,
    name: String,
    label: Option<String>,
    provider_kinds: Vec<String>,
    prefix: String,
    enabled: bool,
    max_concurrency: u64,
    requests_per_minute: u64,
    daily_limit_usd: String,
    weekly_limit_usd: String,
    daily_used_usd: String,
    weekly_used_usd: String,
    daily_resets_at: Option<DateTime<Utc>>,
    weekly_resets_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
}

impl From<ClientKeyRecord> for ClientKeyView {
    fn from(record: ClientKeyRecord) -> Self {
        Self {
            id: record.id.to_string(),
            name: record.name,
            label: record.label,
            provider_kinds: record
                .provider_kinds
                .into_iter()
                .map(|provider| provider.to_string())
                .collect(),
            prefix: record.prefix,
            enabled: record.enabled,
            max_concurrency: record.limits.max_concurrency,
            requests_per_minute: record.limits.requests_per_minute,
            daily_limit_usd: record.budget.limits.daily_usd.canonical(),
            weekly_limit_usd: record.budget.limits.weekly_usd.canonical(),
            daily_used_usd: record.budget.daily_used_usd.canonical(),
            weekly_used_usd: record.budget.weekly_used_usd.canonical(),
            daily_resets_at: record.budget.daily_resets_at.map(DateTime::from),
            weekly_resets_at: record.budget.weekly_resets_at.map(DateTime::from),
            created_at: record.created_at,
            updated_at: record.updated_at,
            last_used_at: record.last_used_at,
        }
    }
}

/// Client Key 列表响应数据。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientKeyListData {
    items: Vec<ClientKeyView>,
    next_cursor: Option<String>,
    total: u64,
}

impl ClientKeyListData {
    /// 构造 Client Key 列表响应。
    #[must_use]
    pub fn new(items: Vec<ClientKeyView>, next_cursor: Option<String>, total: u64) -> Self {
        Self {
            items,
            next_cursor,
            total,
        }
    }
}

impl TryFrom<ClientKeyPage> for ClientKeyListData {
    type Error = WireValidationError;

    fn try_from(page: ClientKeyPage) -> Result<Self, Self::Error> {
        let next_cursor = page
            .next_cursor
            .map(wire_cursor)
            .transpose()?
            .as_ref()
            .map(encode_client_key_cursor)
            .transpose()?;
        Ok(Self::new(
            page.items.into_iter().map(Into::into).collect(),
            next_cursor,
            page.total,
        ))
    }
}

/// Client Key 创建响应；完整值只允许出现在本次序列化结果中。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedClientKeyData {
    id: String,
    prefix: String,
    plaintext_key: String,
}

impl CreatedClientKeyData {
    /// 构造一次性创建响应。
    #[must_use]
    pub fn new(id: String, prefix: String, plaintext_key: String) -> Self {
        Self {
            id,
            prefix,
            plaintext_key,
        }
    }
}

impl From<CreatedClientKey> for CreatedClientKeyData {
    fn from(created: CreatedClientKey) -> Self {
        Self::new(
            created.secret.record.id.to_string(),
            created.secret.record.prefix.clone(),
            created.secret.expose_for_response().to_owned(),
        )
    }
}

impl fmt::Debug for CreatedClientKeyData {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreatedClientKeyData")
            .field("id", &self.id)
            .field("prefix", &self.prefix)
            .field("plaintext_key", &"[REDACTED]")
            .finish()
    }
}

/// 仅由显式 reveal 返回一次的完整明文 Key。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevealedClientKeyData {
    id: String,
    plaintext_key: String,
}

impl RevealedClientKeyData {
    #[must_use]
    pub fn new(id: String, plaintext_key: String) -> Self {
        Self { id, plaintext_key }
    }
}

impl From<ClientKeySecret> for RevealedClientKeyData {
    fn from(secret: ClientKeySecret) -> Self {
        Self::new(
            secret.record.id.to_string(),
            secret.expose_for_response().to_owned(),
        )
    }
}

impl fmt::Debug for RevealedClientKeyData {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RevealedClientKeyData")
            .field("id", &self.id)
            .field("plaintext_key", &"[REDACTED]")
            .finish()
    }
}

/// Client Key mutation 响应数据。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MutatedClientKeyData {
    id: String,
}

impl MutatedClientKeyData {
    /// 构造 mutation 响应。
    #[must_use]
    pub fn new(id: String) -> Self {
        Self { id }
    }
}

impl From<ClientKeyMutation> for MutatedClientKeyData {
    fn from(mutation: ClientKeyMutation) -> Self {
        Self::new(mutation.id.to_string())
    }
}

/// 解码后的 Client Key 游标字段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientKeyCursorData {
    pub sort: ClientKeySort,
    pub value: ClientKeyCursorValue,
    pub id: String,
}

/// 游标中与排序字段严格对应的最后一行值。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum ClientKeyCursorValue {
    Name(String),
    Enabled(bool),
    CreatedAt(DateTime<Utc>),
    LastUsedAt(Option<DateTime<Utc>>),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CursorWire {
    sort: ClientKeySort,
    value: ClientKeyCursorValue,
    id: String,
}

/// 把 owner 游标编码为不透明 wire 值。
pub fn encode_client_key_cursor(
    cursor: &ClientKeyCursorData,
) -> Result<String, WireValidationError> {
    validate_client_key_cursor(cursor)?;
    let bytes = serde_json::to_vec(&CursorWire {
        sort: cursor.sort,
        value: cursor.value.clone(),
        id: cursor.id.clone(),
    })
    .map_err(|_| WireValidationError::new("cursor"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// 解码并严格校验 Client Key 游标。
pub fn decode_client_key_cursor(encoded: &str) -> Result<ClientKeyCursorData, WireValidationError> {
    if encoded.is_empty() || encoded.len() > MAX_CURSOR_BYTES {
        return Err(WireValidationError::new("cursor"));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| WireValidationError::new("cursor"))?;
    let cursor: CursorWire =
        serde_json::from_slice(&bytes).map_err(|_| WireValidationError::new("cursor"))?;
    let cursor = ClientKeyCursorData {
        sort: cursor.sort,
        value: cursor.value,
        id: cursor.id,
    };
    validate_client_key_cursor(&cursor)?;
    Ok(cursor)
}

fn validate_client_key_cursor(cursor: &ClientKeyCursorData) -> Result<(), WireValidationError> {
    validate_required_text(&cursor.id, "cursor")?;
    let matching = matches!(
        (cursor.sort.field, &cursor.value),
        (ClientKeySortField::Name, ClientKeyCursorValue::Name(value)) if !value.trim().is_empty()
    ) || matches!(
        (cursor.sort.field, &cursor.value),
        (
            ClientKeySortField::Enabled,
            ClientKeyCursorValue::Enabled(_)
        ) | (
            ClientKeySortField::CreatedAt,
            ClientKeyCursorValue::CreatedAt(_)
        ) | (
            ClientKeySortField::LastUsedAt,
            ClientKeyCursorValue::LastUsedAt(_)
        )
    );
    if matching {
        Ok(())
    } else {
        Err(WireValidationError::new("cursor"))
    }
}

const fn domain_sort(sort: ClientKeySort) -> DomainSort {
    DomainSort {
        field: match sort.field {
            ClientKeySortField::Name => DomainSortField::Name,
            ClientKeySortField::Enabled => DomainSortField::Enabled,
            ClientKeySortField::CreatedAt => DomainSortField::CreatedAt,
            ClientKeySortField::LastUsedAt => DomainSortField::LastUsedAt,
        },
        direction: match sort.direction {
            ClientKeySortDirection::Asc => SortDirection::Asc,
            ClientKeySortDirection::Desc => SortDirection::Desc,
        },
    }
}

const fn wire_sort(sort: DomainSort) -> ClientKeySort {
    ClientKeySort {
        field: match sort.field {
            DomainSortField::Name => ClientKeySortField::Name,
            DomainSortField::Enabled => ClientKeySortField::Enabled,
            DomainSortField::CreatedAt => ClientKeySortField::CreatedAt,
            DomainSortField::LastUsedAt => ClientKeySortField::LastUsedAt,
        },
        direction: match sort.direction {
            SortDirection::Asc => ClientKeySortDirection::Asc,
            SortDirection::Desc => ClientKeySortDirection::Desc,
        },
    }
}

fn domain_cursor(cursor: ClientKeyCursorData) -> Result<ClientKeyCursor, WireValidationError> {
    let value = match cursor.value {
        ClientKeyCursorValue::Name(value) => DomainCursorValue::Name(value),
        ClientKeyCursorValue::Enabled(value) => DomainCursorValue::Enabled(value),
        ClientKeyCursorValue::CreatedAt(value) => DomainCursorValue::CreatedAt(value),
        ClientKeyCursorValue::LastUsedAt(value) => DomainCursorValue::LastUsedAt(value),
    };
    Ok(ClientKeyCursor {
        sort: domain_sort(cursor.sort),
        value,
        id: client_key_id(cursor.id, "cursor")?,
    })
}

fn wire_cursor(cursor: ClientKeyCursor) -> Result<ClientKeyCursorData, WireValidationError> {
    let value = match cursor.value {
        DomainCursorValue::Name(value) => ClientKeyCursorValue::Name(value),
        DomainCursorValue::Enabled(value) => ClientKeyCursorValue::Enabled(value),
        DomainCursorValue::CreatedAt(value) => ClientKeyCursorValue::CreatedAt(value),
        DomainCursorValue::LastUsedAt(value) => ClientKeyCursorValue::LastUsedAt(value),
    };
    let cursor = ClientKeyCursorData {
        sort: wire_sort(cursor.sort),
        value,
        id: cursor.id.to_string(),
    };
    validate_client_key_cursor(&cursor)?;
    Ok(cursor)
}

fn client_key_id(
    value: String,
    field: &'static str,
) -> Result<ClientApiKeyId, WireValidationError> {
    ClientApiKeyId::new(value).map_err(|_| WireValidationError::new(field))
}

fn validate_required_text(value: &str, field: &'static str) -> Result<(), WireValidationError> {
    if value.trim().is_empty() {
        return Err(WireValidationError::new(field));
    }
    Ok(())
}

/// 唯一的用户 Key 创建合同；没有分组与上游身份字段。
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateClientKeyRequest {
    name: String,
    label: Option<String>,
    custom_key: Option<String>,
    max_concurrency: u64,
    requests_per_minute: u64,
    daily_limit_usd: Option<String>,
    weekly_limit_usd: Option<String>,
}

impl fmt::Debug for CreateClientKeyRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreateClientKeyRequest")
            .field("custom_key", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl CreateClientKeyRequest {
    pub fn into_command(
        self,
    ) -> Result<gateway_admin::model::client_keys::CreateClientKey, WireValidationError> {
        validate_key_text(&self.name, self.label.as_deref())?;
        let custom_key = self
            .custom_key
            .filter(|value| !value.is_empty())
            .map(gateway_core::policy::PlaintextClientApiKey::new)
            .transpose()
            .map_err(|_| WireValidationError::new("customKey"))?;
        Ok(gateway_admin::model::client_keys::CreateClientKey {
            name: self.name,
            label: self.label,
            custom_key,
            group_ids: Vec::new(),
            openai_client_profile_override: None,
            xai_client_profile_override: None,
            limits: key_rate_limits(self.max_concurrency, self.requests_per_minute)?,
            budget: gateway_core::engine::budget::ClientBudgetLimits {
                daily_usd: parse_budget(self.daily_limit_usd, "dailyLimitUsd")?.unwrap_or_default(),
                weekly_usd: parse_budget(self.weekly_limit_usd, "weeklyLimitUsd")?
                    .unwrap_or_default(),
            },
        })
    }
}

/// 编辑只能修改非凭据属性；省略额度保持原值，明确0表示不额外限制。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateClientKeyRequest {
    id: String,
    name: String,
    label: Option<String>,
    max_concurrency: u64,
    requests_per_minute: u64,
    daily_limit_usd: Option<String>,
    weekly_limit_usd: Option<String>,
}

impl UpdateClientKeyRequest {
    pub fn into_command(
        self,
    ) -> Result<gateway_admin::model::client_keys::UpdateClientKey, WireValidationError> {
        validate_key_text(&self.name, self.label.as_deref())?;
        Ok(gateway_admin::model::client_keys::UpdateClientKey {
            id: client_key_id(self.id, "clientKeyMutationNotFound")?,
            name: self.name,
            label: self.label,
            group_ids: Vec::new(),
            openai_client_profile_override: None,
            xai_client_profile_override: None,
            limits: key_rate_limits(self.max_concurrency, self.requests_per_minute)?,
            daily_limit_usd: parse_budget(self.daily_limit_usd, "dailyLimitUsd")?,
            weekly_limit_usd: parse_budget(self.weekly_limit_usd, "weeklyLimitUsd")?,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClientKeyMutationRequest {
    pub(super) id: String,
}
impl ClientKeyMutationRequest {
    pub fn into_id(self) -> Result<String, WireValidationError> {
        Ok(client_key_id(self.id, "clientKeyMutationNotFound")?.to_string())
    }
}

fn parse_budget(
    value: Option<String>,
    field: &'static str,
) -> Result<Option<gateway_core::metering::Decimal>, WireValidationError> {
    value
        .map(|value| value.parse().map_err(|_| WireValidationError::new(field)))
        .transpose()
}
fn key_rate_limits(
    max_concurrency: u64,
    requests_per_minute: u64,
) -> Result<gateway_core::policy::RateLimits, WireValidationError> {
    i64::try_from(max_concurrency).map_err(|_| WireValidationError::new("maxConcurrency"))?;
    i64::try_from(requests_per_minute)
        .map_err(|_| WireValidationError::new("requestsPerMinute"))?;
    Ok(gateway_core::policy::RateLimits {
        max_concurrency,
        requests_per_minute,
    })
}
fn validate_key_text(name: &str, label: Option<&str>) -> Result<(), WireValidationError> {
    validate_required_text(name, "name")?;
    if name.chars().any(char::is_control) {
        return Err(WireValidationError::new("name"));
    }
    if label.is_some_and(|value| value.trim().is_empty() || value.chars().any(char::is_control)) {
        return Err(WireValidationError::new("label"));
    }
    Ok(())
}
