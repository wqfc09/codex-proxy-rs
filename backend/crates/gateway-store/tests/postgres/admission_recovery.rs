use chrono::{DateTime, Duration, Utc};
use gateway_store::postgres::{
    ClientAdmissionRecentRequest, ClientAdmissionRecovery, ClientAdmissionRecoveryRepository,
    ClientAdmissionRunningRequest, PgClientAdmissionRecoveryRepository,
};
use sqlx::PgPool;

use super::TestDatabase;

#[tokio::test]
async fn recovery_loads_precise_window_and_running_request_facts() {
    let Some(database) = TestDatabase::create("admission_recovery").await else {
        return;
    };
    let now = DateTime::from_timestamp_micros(Utc::now().timestamp_micros())
        .expect("current time is representable at PostgreSQL precision");
    let window_started_at = now - Duration::seconds(60);
    seed_request(
        &database.pool,
        "old-running",
        now - Duration::seconds(120),
        now + Duration::seconds(30),
        "running",
    )
    .await;
    seed_request(
        &database.pool,
        "old-complete",
        now - Duration::seconds(90),
        now - Duration::seconds(30),
        "succeeded",
    )
    .await;
    seed_request(
        &database.pool,
        "recent-complete",
        now - Duration::seconds(20),
        now + Duration::seconds(10),
        "succeeded",
    )
    .await;
    seed_request(
        &database.pool,
        "recent-running",
        now - Duration::seconds(10),
        now + Duration::seconds(40),
        "running",
    )
    .await;

    let repository = PgClientAdmissionRecoveryRepository::new(database.pool.clone());
    let actual = repository
        .load_client_admission_recovery(window_started_at)
        .await
        .expect("load precise admission recovery facts");
    let expected = vec![ClientAdmissionRecovery {
        client_api_key_ref: "key-recovery".to_owned(),
        recent_requests: vec![
            ClientAdmissionRecentRequest {
                model_request_id: "recent-complete".to_owned(),
                started_at: now - Duration::seconds(20),
            },
            ClientAdmissionRecentRequest {
                model_request_id: "recent-running".to_owned(),
                started_at: now - Duration::seconds(10),
            },
        ],
        running_requests: vec![
            ClientAdmissionRunningRequest {
                model_request_id: "old-running".to_owned(),
                deadline_at: now + Duration::seconds(30),
            },
            ClientAdmissionRunningRequest {
                model_request_id: "recent-running".to_owned(),
                deadline_at: now + Duration::seconds(40),
            },
        ],
        user: None,
    }];
    assert_eq!(actual, expected);

    database.close().await;
}

#[tokio::test]
async fn recovery_uses_frozen_request_user_after_owned_key_deletion() {
    let Some(database) = TestDatabase::create("admission_recovery_deleted_key").await else {
        return;
    };
    let now = DateTime::from_timestamp_micros(Utc::now().timestamp_micros())
        .expect("current time is representable at PostgreSQL precision");
    let window_started_at = now - Duration::seconds(60);
    sqlx::query(
        "insert into users (id, username, password_hash, role, enabled, created_at, updated_at)
         values ('recovery_shared_user', 'recovery_shared_user', 'hash', 'user', true, now(), now())",
    )
    .execute(&database.pool)
    .await
    .unwrap();
    for key in ["recovery-key-a", "recovery-key-b"] {
        sqlx::query(
            "insert into client_api_keys (id, name, key, owner_user_id, created_at, updated_at)
             values ($1, $1, $2, 'recovery_shared_user', now(), now())",
        )
        .bind(key)
        .bind(format!("sk_{key:a<43}"))
        .execute(&database.pool)
        .await
        .unwrap();
    }
    seed_owned_request(
        &database.pool,
        "deleted-key-running",
        "recovery-key-a",
        "recovery_shared_user",
        now - Duration::seconds(10),
        now + Duration::seconds(30),
    )
    .await;
    seed_owned_request(
        &database.pool,
        "live-key-running",
        "recovery-key-b",
        "recovery_shared_user",
        now - Duration::seconds(5),
        now + Duration::seconds(35),
    )
    .await;
    sqlx::query("delete from client_api_keys where id='recovery-key-a'")
        .execute(&database.pool)
        .await
        .unwrap();

    let repository = PgClientAdmissionRecoveryRepository::new(database.pool.clone());
    let actual = repository
        .load_client_admission_recovery(window_started_at)
        .await
        .expect("load recovery facts after key deletion");
    assert_eq!(actual.len(), 2);
    for recovery in actual {
        let user = recovery.user.expect("frozen request User recovery");
        assert_eq!(user.user_id, "recovery_shared_user");
        assert_eq!(user.recent_requests.len(), 1);
        assert_eq!(user.running_requests.len(), 1);
    }
    database.close().await;
}

async fn seed_request(
    pool: &PgPool,
    id: &str,
    started_at: DateTime<Utc>,
    deadline_at: DateTime<Utc>,
    outcome: &str,
) {
    let completed_at = (outcome != "running").then_some(started_at + Duration::seconds(1));
    sqlx::query(
        "insert into model_requests (
           id, client_api_key_ref, config_revision, protocol, operation, endpoint,
           client_transport, requested_model_id, outcome,
           started_at, deadline_at, completed_at,
           routing_scope, routing_group_refs, routing_group_names_snapshot
         ) values (
           $1, 'key-recovery', 1, 'openai', 'responses', '/v1/responses',
           'http_sse', 'coding', $2, $3, $4, $5,
           'all', '{}'::text[], '[]'::jsonb
         )",
    )
    .bind(id)
    .bind(outcome)
    .bind(started_at)
    .bind(deadline_at)
    .bind(completed_at)
    .execute(pool)
    .await
    .expect("seed model request recovery fact");
}

async fn seed_owned_request(
    pool: &PgPool,
    id: &str,
    key: &str,
    user_id: &str,
    started_at: DateTime<Utc>,
    deadline_at: DateTime<Utc>,
) {
    sqlx::query(
        "insert into model_requests (
           id, client_api_key_ref, user_id, config_revision, protocol, operation, endpoint,
           client_transport, requested_model_id, outcome, started_at, deadline_at,
           routing_scope, routing_group_refs, routing_group_names_snapshot
         ) values (
           $1, $2, $3, 1, 'openai', 'responses', '/v1/responses',
           'http_sse', 'coding', 'running', $4, $5,
           'all', '{}'::text[], '[]'::jsonb
         )",
    )
    .bind(id)
    .bind(key)
    .bind(user_id)
    .bind(started_at)
    .bind(deadline_at)
    .execute(pool)
    .await
    .expect("seed owned model request recovery fact");
}
