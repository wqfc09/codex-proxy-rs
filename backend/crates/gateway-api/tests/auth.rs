use std::sync::Arc;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use serde_json::{Value, json};
use tower::ServiceExt as _;

use crate::support::{auth_app, cookie_request, empty_request, json_request, response_json};

fn legacy_key_cookie(fixture: &Arc<crate::admin::MemoryAuthStore>) -> String {
    fixture.insert_legacy_key_session("legacy-key");
    "cpr_session=legacy-key".to_owned()
}

fn session_cookie(response: &axum::response::Response) -> String {
    response.headers()[header::SET_COOKIE]
        .to_str()
        .expect("cookie")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_owned()
}

async fn login(
    app: &axum::Router,
    body: Value,
    previous: Option<&str>,
) -> axum::response::Response {
    let mut request = json_request(Method::POST, "/api/auth/login", body);
    if let Some(cookie) = previous {
        request
            .headers_mut()
            .insert(header::COOKIE, cookie.parse().expect("cookie"));
    }
    app.clone().oneshot(request).await.expect("login response")
}

async fn get(app: &axum::Router, path: &str, cookie: &str) -> axum::response::Response {
    app.clone()
        .oneshot(cookie_request(Method::GET, path, cookie))
        .await
        .expect("GET response")
}

async fn change_password(
    app: &axum::Router,
    cookie: Option<&str>,
    current: &str,
    new: &str,
) -> axum::response::Response {
    let mut request = json_request(
        Method::POST,
        "/api/auth/password",
        json!({"currentPassword": current, "newPassword": new}),
    );
    if let Some(cookie) = cookie {
        request
            .headers_mut()
            .insert(header::COOKIE, cookie.parse().expect("cookie"));
    }
    app.clone()
        .oneshot(request)
        .await
        .expect("password response")
}

#[tokio::test]
async fn account_login_config_and_status_use_canonical_user_identity() {
    let (app, fixture) = auth_app().await;
    let config = response_json(
        app.clone()
            .oneshot(empty_request(Method::GET, "/api/auth/config"))
            .await
            .expect("auth config response"),
    )
    .await;
    assert_eq!(
        config["data"],
        json!({"turnstileEnabled": false, "turnstileSiteKey": null})
    );
    let response = login(
        &app,
        json!({"username": "admin_1", "password": "strong-admin-password"}),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = session_cookie(&response);
    let data = response_json(response).await["data"].clone();
    assert_eq!(data["role"], "admin");
    assert!(data["expiresAt"].is_string());
    assert_eq!(
        response_json(get(&app, "/api/auth/status", &cookie).await).await["data"]["session"]["role"],
        "admin"
    );
    let key = login(&app, json!({"mode": "key", "apiKey": "legacy"}), None).await;
    assert_eq!(key.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!key.headers().contains_key(header::SET_COOKIE));
    let legacy = legacy_key_cookie(&fixture);
    assert_eq!(
        response_json(get(&app, "/api/auth/status", &legacy).await).await["data"],
        json!({"authenticated": false, "session": null})
    );
}

#[tokio::test]
async fn password_change_revokes_all_admin_sessions_and_keeps_legacy_key_outside_browser_auth() {
    let (app, fixture) = auth_app().await;
    let body = json!({"username": "admin_1", "password": "strong-admin-password"});
    let first = session_cookie(&login(&app, body.clone(), None).await);
    let second = session_cookie(&login(&app, body, None).await);
    let legacy = legacy_key_cookie(&fixture);
    let response = change_password(
        &app,
        Some(&first),
        "strong-admin-password",
        "new-strong-admin-password",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert!(
        response.headers()[header::SET_COOKIE]
            .to_str()
            .expect("clear cookie")
            .contains("Max-Age=0")
    );
    for cookie in [&first, &second] {
        assert_eq!(
            get(&app, "/api/admin/system/version", cookie)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        response_json(get(&app, "/api/auth/status", &legacy).await).await["data"]["authenticated"],
        false
    );
    assert_eq!(
        login(
            &app,
            json!({"username": "admin_1", "password": "strong-admin-password"}),
            None
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        login(
            &app,
            json!({"username": "admin_1", "password": "new-strong-admin-password"}),
            None
        )
        .await
        .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn password_change_requires_admin_session_and_validation_keeps_it_alive() {
    let (app, fixture) = auth_app().await;
    assert_eq!(
        change_password(&app, None, "strong-admin-password", "new-strong-password")
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let legacy = legacy_key_cookie(&fixture);
    assert_eq!(
        change_password(
            &app,
            Some(&legacy),
            "strong-admin-password",
            "new-strong-password"
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    let admin = session_cookie(
        &login(
            &app,
            json!({"username": "admin_1", "password": "strong-admin-password"}),
            None,
        )
        .await,
    );
    for (current, new) in [
        ("wrong-current-password", "new-strong-password"),
        ("strong-admin-password", "short"),
        ("strong-admin-password", "strong-admin-password"),
    ] {
        assert_eq!(
            change_password(&app, Some(&admin), current, new)
                .await
                .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            get(&app, "/api/admin/system/version", &admin)
                .await
                .status(),
            StatusCode::OK
        );
    }
}

#[tokio::test]
async fn password_change_audit_failure_preserves_password_and_session() {
    let (app, fixture) = auth_app().await;
    let admin = session_cookie(
        &login(
            &app,
            json!({"username": "admin_1", "password": "strong-admin-password"}),
            None,
        )
        .await,
    );
    fixture.fail_audit(true);
    assert_eq!(
        change_password(
            &app,
            Some(&admin),
            "strong-admin-password",
            "new-strong-password"
        )
        .await
        .status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        get(&app, "/api/admin/system/version", &admin)
            .await
            .status(),
        StatusCode::OK
    );
    fixture.fail_audit(false);
    assert_eq!(
        login(
            &app,
            json!({"username": "admin_1", "password": "strong-admin-password"}),
            None
        )
        .await
        .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn account_login_rotates_previous_account_session_and_rejects_key_login() {
    let (app, fixture) = auth_app().await;
    let first = session_cookie(
        &login(
            &app,
            json!({"username": "admin_1", "password": "strong-admin-password"}),
            None,
        )
        .await,
    );
    let second_response = login(
        &app,
        json!({"username": "admin_1", "password": "strong-admin-password"}),
        Some(&first),
    )
    .await;
    assert_eq!(second_response.status(), StatusCode::OK);
    let second = session_cookie(&second_response);
    assert_ne!(first, second);
    assert_eq!(
        get(&app, "/api/admin/system/version", &first)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        get(&app, "/api/admin/system/version", &second)
            .await
            .status(),
        StatusCode::OK
    );
    let rejected = login(
        &app,
        json!({"mode": "key", "apiKey": "legacy"}),
        Some(&second),
    )
    .await;
    assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!rejected.headers().contains_key(header::SET_COOKIE));
    assert_eq!(
        get(&app, "/api/admin/system/version", &second)
            .await
            .status(),
        StatusCode::OK
    );
    let legacy = legacy_key_cookie(&fixture);
    assert_eq!(
        response_json(get(&app, "/api/auth/status", &legacy).await).await["data"]["authenticated"],
        false
    );
}

#[tokio::test]
async fn canonical_user_login_is_user_scoped_and_cannot_enter_admin_routes() {
    let (app, fixture) = auth_app().await;
    fixture.insert_user(
        "user-1",
        "alice",
        gateway_admin::model::users::UserRole::User,
    );
    let response = login(
        &app,
        json!({"username": "alice", "password": "strong-admin-password"}),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = session_cookie(&response);
    assert_eq!(response_json(response).await["data"]["role"], "user");
    assert_eq!(
        get(&app, "/api/admin/system/version", &cookie)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        get(&app, "/api/user/me", &cookie).await.status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn legacy_login_shapes_are_rejected_without_replacing_current_session() {
    let (app, _) = auth_app().await;
    let cookie = session_cookie(
        &login(
            &app,
            json!({"username": "admin_1", "password": "strong-admin-password"}),
            None,
        )
        .await,
    );
    for body in [
        json!({"type": "key", "apiKey": "legacy"}),
        json!({"type": "admin", "password": "strong-admin-password"}),
        json!({"mode": "key", "type": "key", "apiKey": "legacy"}),
        json!({"mode": "admin", "type": "admin", "password": "strong-admin-password"}),
    ] {
        let response = login(&app, body, Some(&cookie)).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert!(!response.headers().contains_key(header::SET_COOKIE));
        assert!(
            !response_json(response)
                .await
                .to_string()
                .contains("strong-admin-password")
        );
    }
    assert_eq!(
        get(&app, "/api/admin/system/version", &cookie)
            .await
            .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn legacy_key_session_cannot_read_or_mutate_admin_resources() {
    let (app, fixture) = auth_app().await;
    let cookie = legacy_key_cookie(&fixture);
    for path in [
        "/api/admin/system/version",
        "/api/admin/accounts",
        "/api/admin/users",
        "/api/admin/settings",
    ] {
        let response = get(&app, path, &cookie).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
        assert_eq!(response_json(response).await["code"], 40301);
    }
    let mut request = json_request(
        Method::POST,
        "/api/admin/settings/admin-api-key/regenerate",
        json!({}),
    );
    request
        .headers_mut()
        .insert(header::COOKIE, cookie.parse().expect("cookie"));
    assert_eq!(
        app.clone()
            .oneshot(request)
            .await
            .expect("mutation response")
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        response_json(get(&app, "/api/auth/status", &cookie).await).await["data"],
        json!({"authenticated": false, "session": null})
    );
}

#[tokio::test]
async fn data_plane_keys_and_browser_sessions_are_not_interchangeable() {
    let (app, fixture) = auth_app().await;
    let cookie = legacy_key_cookie(&fixture);
    assert_eq!(
        get(&app, "/v1/models", &cookie).await.status(),
        StatusCode::UNAUTHORIZED
    );
    for (name, value) in [
        (header::AUTHORIZATION.as_str(), "Bearer legacy"),
        ("x-api-key", "legacy"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::get("/api/auth/status")
                    .header(name, value)
                    .body(Body::empty())
                    .expect("header request"),
            )
            .await
            .expect("status response");
        assert_eq!(
            response_json(response).await["data"]["authenticated"],
            false
        );
    }
    assert_eq!(
        response_json(get(&app, "/api/auth/status", "cpr_session=legacy").await).await["data"]["authenticated"],
        false
    );
}

#[test]
fn login_wire_never_prints_password_or_turnstile_token() {
    let request: gateway_api::auth::LoginRequest = serde_json::from_value(json!({"username": "admin_1", "password": "password-secret", "turnstileToken": "token-secret"})).expect("login request");
    let debug = format!("{request:?}");
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains("password-secret"));
    assert!(!debug.contains("token-secret"));
}

#[tokio::test]
async fn account_login_should_issue_and_clear_a_unified_cookie() {
    let (app, _) = auth_app().await;
    let login = login(
        &app,
        json!({"username": "admin_1", "password": "strong-admin-password"}),
        None,
    )
    .await;
    assert_eq!(login.status(), StatusCode::OK);
    assert_eq!(login.headers()[header::CACHE_CONTROL], "no-store");
    let set_cookie = login.headers()[header::SET_COOKIE]
        .to_str()
        .expect("session cookie")
        .to_owned();
    assert!(set_cookie.starts_with("cpr_session=session_"));
    assert!(set_cookie.contains("Path=/;"));
    assert!(set_cookie.contains("; Secure; HttpOnly; SameSite=Lax"));
    assert!(set_cookie.contains("; Max-Age="));
    let cookie = set_cookie
        .split(';')
        .next()
        .expect("cookie pair")
        .to_owned();
    assert_eq!(response_json(login).await["data"]["role"], "admin");
    let logout = app
        .clone()
        .oneshot(cookie_request(Method::POST, "/api/auth/logout", &cookie))
        .await
        .expect("logout response");
    assert_eq!(logout.status(), StatusCode::OK);
    let cleared = logout.headers()[header::SET_COOKIE]
        .to_str()
        .expect("cleared cookie");
    assert!(cleared.starts_with("cpr_session=;"));
    assert!(cleared.contains("Max-Age=0"));
    assert_eq!(
        response_json(get(&app, "/api/auth/status", &cookie).await).await["data"]["authenticated"],
        false
    );
}

#[tokio::test]
async fn removed_auth_routes_do_not_accept_old_cookie_names() {
    let (app, _) = auth_app().await;
    for path in ["/api/admin/auth/login", "/api/auth/unknown"] {
        let response = app
            .clone()
            .oneshot(empty_request(Method::GET, path))
            .await
            .expect("route response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response_json(response).await["code"], 40401);
    }
    for old_name in ["cpr_admin_session", "cpr_client_session"] {
        assert_eq!(
            response_json(get(&app, "/api/auth/status", &format!("{old_name}=legacy")).await).await
                ["data"]["authenticated"],
            false
        );
    }
}
