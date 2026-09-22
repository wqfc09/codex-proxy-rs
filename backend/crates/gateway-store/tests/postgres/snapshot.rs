use gateway_core::{
    policy::{ClientApiKeyId, PlaintextClientApiKey, RateLimits},
    routing::snapshot::SnapshotStorePort,
};
use gateway_store::postgres::{
    ClientApiKeySnapshot, PgRuntimeSnapshotRepository, RuntimeSnapshotRepository,
};

use super::TestDatabase;

#[test]
fn snapshot_client_policy_contains_only_common_limits() {
    let policy = ClientApiKeySnapshot {
        request_profiles: Default::default(),
        id: ClientApiKeyId::new("key-1").expect("client key ID"),
        plaintext_key: PlaintextClientApiKey::new("sk_snapshot_secret").expect("plaintext key"),
        group_ids: Vec::new(),
        limits: RateLimits {
            max_concurrency: 3,
            requests_per_minute: 60,
        },
    };
    assert_eq!(policy.limits.max_concurrency, 3);
    assert!(policy.group_ids.is_empty());
    assert!(!format!("{policy:?}").contains("sk_snapshot_secret"));
}

#[tokio::test]
async fn runtime_snapshot_loads_enabled_plaintext_key_without_debug_exposure() {
    let Some(database) = TestDatabase::create("client_snapshot").await else {
        return;
    };
    let plaintext = format!("sk_{}", "s".repeat(43));
    sqlx::query(
        "insert into client_api_keys (
           id, name, key, enabled, max_concurrency, requests_per_minute, created_at, updated_at
         ) values ('key_snapshot', 'snapshot', $1, true, 2, 60, now(), now())",
    )
    .bind(&plaintext)
    .execute(&database.pool)
    .await
    .expect("seed client API key");
    let snapshot = PgRuntimeSnapshotRepository::new(database.pool.clone())
        .load_runtime_snapshot()
        .await
        .expect("load runtime snapshot");
    assert_eq!(snapshot.client_api_keys.len(), 1);
    assert!(snapshot.client_api_keys[0].group_ids.is_empty());
    assert_eq!(
        snapshot.client_api_keys[0].plaintext_key.expose_for_auth(),
        plaintext
    );
    assert!(!format!("{snapshot:?}").contains(&plaintext));
    database.close().await;
}

#[tokio::test]
async fn runtime_snapshot_suppresses_enabled_key_owned_by_disabled_user() {
    let Some(database) = TestDatabase::create("client_snapshot_disabled_owner").await else {
        return;
    };
    sqlx::query(
        "insert into users (id, username, password_hash, role, enabled, created_at, updated_at)
         values ('user_disabled_snapshot', 'disabled_snapshot', 'hash', 'user', false, now(), now())",
    )
    .execute(&database.pool)
    .await
    .expect("seed disabled user");
    sqlx::query(
        "insert into client_api_keys
           (id, owner_user_id, name, key, enabled, max_concurrency, requests_per_minute, created_at, updated_at)
         values ('key_disabled_owner_snapshot', 'user_disabled_snapshot', 'owned', 'sk_disabled_owner_snapshot', true, 0, 0, now(), now())",
    )
    .execute(&database.pool)
    .await
    .expect("seed enabled owned key");
    sqlx::query(
        "insert into users (id, username, password_hash, role, enabled, created_at, updated_at)
         values ('user_enabled_snapshot', 'enabled_snapshot', 'hash', 'user', true, now(), now())",
    )
    .execute(&database.pool)
    .await
    .expect("seed enabled user");
    sqlx::query(
        "insert into client_api_keys
           (id, owner_user_id, name, key, enabled, max_concurrency, requests_per_minute, created_at, updated_at)
         values ('key_enabled_owner_snapshot', 'user_enabled_snapshot', 'enabled-owned', 'sk_enabled_owner_snapshot', true, 0, 0, now(), now())",
    )
    .execute(&database.pool)
    .await
    .expect("seed enabled owned key");
    sqlx::query(
        "insert into client_api_keys
           (id, name, key, enabled, max_concurrency, requests_per_minute, created_at, updated_at)
         values ('key_legacy_snapshot', 'legacy', 'sk_legacy_snapshot', true, 0, 0, now(), now())",
    )
    .execute(&database.pool)
    .await
    .expect("seed legacy key");
    let result = PgRuntimeSnapshotRepository::new(database.pool.clone())
        .load_snapshot_facts()
        .await;
    let facts = result.expect("disabled owner key must be suppressed, not poison snapshot");
    let key_ids = facts
        .client_policies()
        .map(|policy| policy.key_id().as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        key_ids,
        vec!["key_enabled_owner_snapshot", "key_legacy_snapshot"]
    );
    database.close().await;
}

#[tokio::test]
async fn runtime_snapshot_suppresses_invalid_owned_policy_without_breaking_legacy_keys() {
    let Some(database) = TestDatabase::create("client_snapshot_no_base").await else {
        return;
    };
    sqlx::query(
        "insert into users (id, username, password_hash, role, enabled, created_at, updated_at)
         values ('user_no_base_snapshot', 'no_base_snapshot', 'hash', 'user', true, now(), now())",
    )
    .execute(&database.pool)
    .await
    .expect("seed user");
    sqlx::query("update subscription_plans set enabled=false where is_base")
        .execute(&database.pool)
        .await
        .expect("disable base plan");
    sqlx::query(
        "insert into client_api_keys
           (id, owner_user_id, name, key, enabled, max_concurrency, requests_per_minute, created_at, updated_at)
         values ('key_no_base_snapshot', 'user_no_base_snapshot', 'owned', 'sk_no_base_snapshot', true, 0, 0, now(), now())",
    )
    .execute(&database.pool)
    .await
    .expect("seed owned key");
    sqlx::query("insert into client_api_keys(id,name,key,enabled,created_at,updated_at) values('key_still_legacy','legacy','sk_still_legacy',true,now(),now())")
        .execute(&database.pool).await.unwrap();
    let facts = PgRuntimeSnapshotRepository::new(database.pool.clone())
        .load_snapshot_facts()
        .await
        .unwrap();
    let ids: Vec<_> = facts
        .client_policies()
        .map(|p| p.key_id().as_str())
        .collect();
    assert_eq!(
        ids,
        vec!["key_still_legacy"],
        "invalid User must be absent, not converted to ownerless or blocking legacy publication"
    );
    database.close().await;
}

#[tokio::test]
async fn billing_policy_preserves_unlimited_null_and_real_zero_limits() {
    let Some(database) = TestDatabase::create("snapshot_budget_null_zero").await else {
        return;
    };
    sqlx::query("insert into users(id,username,password_hash,role,created_at,updated_at) values ('user_budget_snapshot','budget-snapshot','test-only','user',now(),now())")
        .execute(&database.pool).await.unwrap();
    sqlx::query("insert into client_api_keys(id,owner_user_id,name,key,enabled,created_at,updated_at) values ('key_budget_snapshot','user_budget_snapshot','budget','sk_test_budget_snapshot',true,now(),now())")
        .execute(&database.pool).await.unwrap();
    for (value, expected) in [
        (None, None),
        (Some("0"), Some(gateway_core::metering::Decimal::ZERO)),
    ] {
        sqlx::query(
            "update subscription_plans set daily_limit_usd=$1::text::numeric where is_base",
        )
        .bind(value)
        .execute(&database.pool)
        .await
        .unwrap();
        let snapshot = gateway_core::routing::snapshot::RuntimeSnapshotCompiler::new(
            std::sync::Arc::new(PgRuntimeSnapshotRepository::new(database.pool.clone())),
            std::sync::Arc::new(gateway_core::engine::provider::ProviderRegistry::default()),
        )
        .compile()
        .await
        .unwrap();
        let policy = snapshot
            .client_policies()
            .find(|p| p.key_id().as_str() == "key_budget_snapshot")
            .unwrap();
        assert_eq!(
            policy.billing_policy().unwrap().budget_limits().daily_usd,
            expected
        );
    }
    database.close().await;
}

#[tokio::test]
async fn malformed_owned_identity_only_suppresses_its_own_keys() {
    let Some(database) = TestDatabase::create("snapshot_bad_owner").await else {
        return;
    };
    let malformed_id = "bad\nowner".to_owned();
    for (id, name, key) in [
        (malformed_id.as_str(), "bad", "key_bad_owner"),
        ("user_good_owner", "good", "key_good_owner"),
    ] {
        sqlx::query("insert into users(id,username,password_hash,role,created_at,updated_at) values($1,$2,'test-only','user',now(),now())")
            .bind(id).bind(name).execute(&database.pool).await.unwrap();
        sqlx::query("insert into client_api_keys(id,owner_user_id,name,key,enabled,created_at,updated_at) values($1,$2,$1,$1,true,now(),now())")
            .bind(key).bind(id).execute(&database.pool).await.unwrap();
    }
    let facts = PgRuntimeSnapshotRepository::new(database.pool.clone())
        .load_snapshot_facts()
        .await
        .unwrap();
    let ids: Vec<_> = facts
        .client_policies()
        .map(|p| p.key_id().as_str())
        .collect();
    assert_eq!(ids, vec!["key_good_owner"]);
    database.close().await;
}
