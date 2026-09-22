use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use gateway_admin::model::users::UserRole;
use gateway_api::admin::users;
use serde_json::{Value, json};
use tower::ServiceExt as _;

use super::{AdminTestFixture, AdminTestState};

async fn get(fixture: &AdminTestFixture, path: &str) -> (StatusCode, Value) {
    let response = users::router::<AdminTestState>()
        .with_state(fixture.state())
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(path)
                .header(header::COOKIE, "cpr_session=valid-user-admin")
                .header("x-request-id", "req-user-contract-get")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let value = serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
        .unwrap();
    (status, value)
}

async fn post(fixture: &AdminTestFixture, path: &str, payload: Value) -> (StatusCode, Value) {
    let response = users::router::<AdminTestState>()
        .with_state(fixture.state())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header(header::COOKIE, "cpr_session=valid-user-admin")
                .header(header::CONTENT_TYPE, "application/json")
                .header("x-request-id", "req-user-contract")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let value = serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
        .unwrap();
    (status, value)
}

#[tokio::test]
async fn user_key_identity_lists_owned_keys_and_updates_one_key_override() {
    let fixture = AdminTestFixture::new().await;
    fixture.auth.insert_session("valid-user-admin");
    let now = chrono::Utc::now();
    *fixture.client_key.lock().unwrap() =
        Some(gateway_admin::model::client_keys::ClientKeyRecord {
            openai_client_profile_override: None,
            xai_client_profile_override: None,
            id: gateway_core::policy::ClientApiKeyId::new("key_identity_test").unwrap(),
            name: "desktop-key".to_owned(),
            label: Some("Mac".to_owned()),
            groups: Vec::new(),
            provider_kinds: Vec::new(),
            prefix: "sk_testprefix".to_owned(),
            enabled: true,
            limits: gateway_core::policy::RateLimits::unlimited(),
            budget: Default::default(),
            last_used_at: None,
            created_at: now,
            updated_at: now,
        });

    let (status, before) = get(&fixture, "/api/admin/users/key-identity?userId=admin_1").await;
    assert_eq!(status, StatusCode::OK, "{before}");
    assert_eq!(before["data"]["keyCount"], 1);
    assert_eq!(before["data"]["keys"][0]["name"], "desktop-key");
    assert!(before["data"]["keys"][0]["openaiClientProfileOverride"].is_null());

    let (status, _) = post(
        &fixture,
        "/api/admin/users/key-identity/key/update",
        json!({
            "userId":"admin_1",
            "keyId":"key_identity_test",
            "openaiClientProfileOverride":{"client":"cli","platform":"linux","versionMode":"latest"},
            "xaiClientProfileOverride":null
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, after) = get(&fixture, "/api/admin/users/key-identity?userId=admin_1").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        after["data"]["keys"][0]["openaiClientProfileOverride"]["versionMode"],
        "latest"
    );
}

#[tokio::test]
async fn create_user_request_does_not_accept_role_selection() {
    let fixture = AdminTestFixture::new().await;
    fixture.auth.insert_session("valid-user-admin");
    let (status, value) = post(
        &fixture,
        "/api/admin/users/create",
        json!({
            "username":"alice","password":"test-password","enabled":true
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(value["data"]["role"], "user");
    assert_eq!(value["data"]["maxConcurrency"], 0);
    assert_eq!(value["data"]["requestsPerMinute"], 0);
    let (status, _) = post(
        &fixture,
        "/api/admin/users/create",
        json!({
            "username":"elevated","password":"test-password","enabled":true,"role":"admin"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn account_rate_patch_distinguishes_omitted_null_and_zero() {
    let fixture = AdminTestFixture::new().await;
    fixture.auth.insert_session("valid-user-admin");
    fixture
        .auth
        .insert_user("user_patch", "patch", UserRole::User);
    let (status, initial) = post(
        &fixture,
        "/api/admin/users/update",
        json!({
            "userId":"user_patch","maxConcurrency":3,"requestsPerMinute":5
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(initial["data"]["maxConcurrency"], 3);
    let (status, omitted) = post(
        &fixture,
        "/api/admin/users/update",
        json!({"userId":"user_patch"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(omitted["data"]["maxConcurrency"], 3);
    assert_eq!(omitted["data"]["requestsPerMinute"], 5);
    let (status, explicit) = post(
        &fixture,
        "/api/admin/users/update",
        json!({
            "userId":"user_patch","maxConcurrency":0,"requestsPerMinute":null
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(explicit["data"]["maxConcurrency"], 0);
    assert!(explicit["data"]["requestsPerMinute"].is_null());
}

#[tokio::test]
async fn user_mutations_require_a_static_action_body_user_id() {
    let fixture = AdminTestFixture::new().await;
    fixture.auth.insert_session("valid-user-admin");
    for (path, payload) in [
        ("/api/admin/users/update", json!({"maxConcurrency":0})),
        (
            "/api/admin/users/reset-password",
            json!({"userId":null,"password":"new-password"}),
        ),
        (
            "/api/admin/users/revoke-sessions",
            json!({"userId":"user_1","unexpected":true}),
        ),
    ] {
        assert_eq!(
            post(&fixture, path, payload).await.0,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{path}"
        );
    }
}

#[tokio::test]
async fn revoked_session_response_reports_invalidation_without_a_count() {
    let fixture = AdminTestFixture::new().await;
    fixture.auth.insert_session("valid-user-admin");
    fixture
        .auth
        .insert_user("user_revoke", "revoke", UserRole::User);
    let (status, value) = post(
        &fixture,
        "/api/admin/users/revoke-sessions",
        json!({"userId":"user_revoke"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["data"], json!({"invalidated":true}));
}
