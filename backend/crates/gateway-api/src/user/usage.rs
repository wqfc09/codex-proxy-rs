//! 普通用户 Usage HTTP 边界；user scope 由 Session 身份强制注入。

use axum::{Router, extract::State, http::StatusCode, response::IntoResponse, routing::get};
use chrono::{DateTime, Utc};
use gateway_admin::model::{
    PageSize,
    observability::{
        RequestOutcome, UserUsageBreakdown, UserUsageDailyPoint, UserUsageFilter, UserUsagePage,
        UserUsageQuery, UserUsageRecord, UserUsageSummary,
    },
};
use serde::{Deserialize, Serialize};

use crate::admin::{
    AdminEnvelope, AdminError, AdminQuery, AdminResponse,
    observability::{DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE, parse_status, request_outcome, usage_range},
    wire::map_admin_service_error,
};

use super::{UserAuth, UserSessionState};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UserUsageQueryWire {
    current_page: Option<u32>,
    page_size: Option<u16>,
    client_api_key_id: Option<String>,
    model: Option<String>,
    outcome: Option<String>,
    status_code: Option<i64>,
    start_time: Option<String>,
    end_time: Option<String>,
}

impl UserUsageQueryWire {
    fn filter(&self) -> Result<UserUsageFilter, AdminError> {
        Ok(UserUsageFilter {
            client_api_key_ref: non_empty(self.client_api_key_id.clone()),
            model: non_empty(self.model.clone()),
            outcome: request_outcome(self.outcome.clone()).map_err(|_| invalid_query("outcome"))?,
            status_code: parse_status(self.status_code).map_err(|_| invalid_query("statusCode"))?,
        })
    }

    fn command(&self) -> Result<UserUsageQuery, AdminError> {
        let current_page = self.current_page.unwrap_or(1);
        if current_page == 0 {
            return Err(invalid_query("currentPage"));
        }
        let page_size = self.page_size.unwrap_or(DEFAULT_PAGE_SIZE);
        if page_size == 0 || page_size > MAX_PAGE_SIZE {
            return Err(invalid_query("pageSize"));
        }
        Ok(UserUsageQuery {
            range: usage_range(self.start_time.as_deref(), self.end_time.as_deref())
                .map_err(|_| invalid_query("timeRange"))?,
            filter: self.filter()?,
            current_page,
            page_size: PageSize::new(page_size).map_err(|_| invalid_query("pageSize"))?,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UserUsageDetailQuery {
    id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UserUsagePageView {
    items: Vec<UserUsageRecordView>,
    current_page: u32,
    page_size: u16,
    total: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UserUsageRecordView {
    id: String,
    client_api_key_id: String,
    client_api_key_name: String,
    operation: String,
    request_kind: Option<String>,
    requested_model: Option<String>,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cached_tokens: Option<u64>,
    cache_write_tokens: Option<u64>,
    reasoning_tokens: Option<u64>,
    image_input_tokens: Option<u64>,
    image_output_tokens: Option<u64>,
    total_tokens: Option<u64>,
    image_requested_size: Option<String>,
    image_requested_count: Option<u64>,
    image_output_size: Option<String>,
    image_count: Option<u64>,
    image_billing_tier: Option<String>,
    downstream_rate_multiplier: Option<String>,
    billed_amount_usd: Option<String>,
    outcome: String,
    client_status_code: Option<u16>,
    latency_ms: Option<u64>,
    started_at: DateTime<Utc>,
    completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UserUsageDailyPointView {
    bucket_start: DateTime<Utc>,
    request_count: u64,
    success_count: u64,
    failure_count: u64,
    total_tokens: u64,
    billed_usd: Option<String>,
    billed_known_count: u64,
    billed_unknown_count: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UserUsageBreakdownView {
    id: Option<String>,
    name: String,
    request_count: u64,
    total_tokens: u64,
    is_other: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UserUsageSummaryView {
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
    request_count: u64,
    success_count: u64,
    failure_count: u64,
    total_tokens: u64,
    billed_usd: Option<String>,
    billed_known_count: u64,
    billed_unknown_count: u64,
    input_tokens: u64,
    output_tokens: u64,
    cached_tokens: u64,
    average_latency_ms: Option<u64>,
    trend_granularity: &'static str,
    trend: Vec<UserUsageDailyPointView>,
    daily: Vec<UserUsageDailyPointView>,
    models: Vec<UserUsageBreakdownView>,
    client_keys: Vec<UserUsageBreakdownView>,
}

pub(super) fn router<S>() -> Router<S>
where
    S: UserSessionState + Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/api/user/usage/records", get(records::<S>))
        .route("/api/user/usage/records/detail", get(detail::<S>))
        .route("/api/user/usage/summary", get(summary::<S>))
}

async fn records<S>(
    auth: UserAuth,
    State(state): State<S>,
    AdminQuery(query): AdminQuery<UserUsageQueryWire>,
) -> Result<impl IntoResponse, AdminError>
where
    S: UserSessionState + Send + Sync,
{
    let page = state
        .user_services()
        .observability()
        .user_usage_records(&auth.user.id, query.command()?)
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(page_view(page)),
    ))
}

async fn detail<S>(
    auth: UserAuth,
    State(state): State<S>,
    AdminQuery(query): AdminQuery<UserUsageDetailQuery>,
) -> Result<impl IntoResponse, AdminError>
where
    S: UserSessionState + Send + Sync,
{
    let request_id = query.id.trim();
    if request_id.is_empty() || request_id.chars().any(char::is_control) {
        return Err(invalid_query("id"));
    }
    let record = state
        .user_services()
        .observability()
        .user_usage_record_detail(&auth.user.id, request_id)
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(record_view(record)),
    ))
}

async fn summary<S>(
    auth: UserAuth,
    State(state): State<S>,
    AdminQuery(query): AdminQuery<UserUsageQueryWire>,
) -> Result<impl IntoResponse, AdminError>
where
    S: UserSessionState + Send + Sync,
{
    let range = usage_range(query.start_time.as_deref(), query.end_time.as_deref())
        .map_err(|_| invalid_query("timeRange"))?;
    let result = state
        .user_services()
        .observability()
        .user_usage_summary(&auth.user.id, range, query.filter()?)
        .await
        .map_err(map_admin_service_error)?;
    Ok(AdminResponse::new(
        StatusCode::OK,
        AdminEnvelope::ok(summary_view(result)),
    ))
}

fn page_view(page: UserUsagePage) -> UserUsagePageView {
    UserUsagePageView {
        items: page.items.into_iter().map(record_view).collect(),
        current_page: page.current_page,
        page_size: page.page_size,
        total: page.total,
    }
}

fn record_view(record: UserUsageRecord) -> UserUsageRecordView {
    UserUsageRecordView {
        id: record.id,
        client_api_key_id: record.client_api_key_ref,
        client_api_key_name: record.client_api_key_name,
        operation: record.operation,
        request_kind: record.request_kind,
        requested_model: record.requested_model_id,
        input_tokens: record.input_tokens,
        output_tokens: record.output_tokens,
        cached_tokens: record.cached_tokens,
        cache_write_tokens: record.cache_write_tokens,
        reasoning_tokens: record.reasoning_tokens,
        image_input_tokens: record.image_input_tokens,
        image_output_tokens: record.image_output_tokens,
        total_tokens: record.total_tokens,
        image_requested_size: record.image_requested_size,
        image_requested_count: record.image_requested_count,
        image_output_size: record.image_output_size,
        image_count: record.image_count,
        image_billing_tier: record.image_billing_tier,
        downstream_rate_multiplier: record
            .downstream_rate_multiplier
            .map(|value| value.as_str().to_owned()),
        billed_amount_usd: record
            .downstream_billed_amount
            .map(|value| value.as_str().to_owned()),
        outcome: outcome_name(&record.outcome).to_owned(),
        client_status_code: record.client_status_code,
        latency_ms: record.latency_ms,
        started_at: record.started_at,
        completed_at: record.completed_at,
    }
}

fn summary_view(summary: UserUsageSummary) -> UserUsageSummaryView {
    UserUsageSummaryView {
        start_time: summary.range.start,
        end_time: summary.range.end,
        request_count: summary.request_count,
        success_count: summary.success_count,
        failure_count: summary.failure_count,
        total_tokens: summary.total_tokens,
        billed_usd: summary.billed_usd.map(|value| value.as_str().to_owned()),
        billed_known_count: summary.billed_known_count,
        billed_unknown_count: summary.billed_unknown_count,
        input_tokens: summary.input_tokens,
        output_tokens: summary.output_tokens,
        cached_tokens: summary.cached_tokens,
        average_latency_ms: summary.average_latency_ms,
        trend_granularity: summary.trend_granularity,
        trend: summary.trend.into_iter().map(daily_view).collect(),
        daily: summary.daily.into_iter().map(daily_view).collect(),
        models: summary.models.into_iter().map(breakdown_view).collect(),
        client_keys: summary
            .client_keys
            .into_iter()
            .map(breakdown_view)
            .collect(),
    }
}

fn breakdown_view(item: UserUsageBreakdown) -> UserUsageBreakdownView {
    UserUsageBreakdownView {
        id: item.id,
        name: item.name,
        request_count: item.request_count,
        total_tokens: item.total_tokens,
        is_other: item.is_other,
    }
}

fn daily_view(point: UserUsageDailyPoint) -> UserUsageDailyPointView {
    UserUsageDailyPointView {
        bucket_start: point.bucket_start,
        request_count: point.request_count,
        success_count: point.success_count,
        failure_count: point.failure_count,
        total_tokens: point.total_tokens,
        billed_usd: point.billed_usd.map(|value| value.as_str().to_owned()),
        billed_known_count: point.billed_known_count,
        billed_unknown_count: point.billed_unknown_count,
    }
}

fn outcome_name(outcome: &RequestOutcome) -> &str {
    outcome.as_str()
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn invalid_query(field: &'static str) -> AdminError {
    AdminError::bad_request(format!("{field} 不合法"))
}
