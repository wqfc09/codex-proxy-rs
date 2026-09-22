use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use gateway_api::admin::subscription_billing;
use serde_json::json;
use tower::ServiceExt as _;

use super::{AdminTestFixture, AdminTestState};

#[tokio::test]
async fn user_id_wire_fields_are_required_and_reject_unknown_values() {
    let fixture = AdminTestFixture::new().await;
    fixture.auth.insert_session("valid-subscription-session");
    for (method, uri, payload) in [
        (
            "POST",
            "/api/admin/users/subscription/grant",
            Some(json!({"planId":"plan_1","durationDays":30})),
        ),
        (
            "GET",
            "/api/admin/users/groups?userId=user_1&other=true",
            None,
        ),
        ("GET", "/api/admin/users/billing/topups", None),
    ] {
        let response = subscription_billing::router::<AdminTestState>()
            .with_state(fixture.state())
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header(header::COOKIE, "cpr_session=valid-subscription-session")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header("x-request-id", "req-user-contract")
                    .body(payload.map_or_else(Body::empty, |value| Body::from(value.to_string())))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            if method == "GET" {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            },
            "{uri}"
        );
    }
}

#[tokio::test]
async fn update_plan_rejects_unknown_fields_without_flattening() {
    let fixture = AdminTestFixture::new().await;
    fixture.auth.insert_session("valid-subscription-session");
    let response = subscription_billing::router::<AdminTestState>()
        .with_state(fixture.state())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/admin/subscription-plans/update")
                .header(header::COOKIE, "cpr_session=valid-subscription-session")
                .header(header::CONTENT_TYPE, "application/json")
                .header("x-request-id", "req-user-contract")
                .body(Body::from(
                    json!({"id":"plan_1","name":"Plan","description":null,
                "dailyLimitUsd":null,"weeklyLimitUsd":null,"monthlyLimitUsd":null,"other":true})
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
