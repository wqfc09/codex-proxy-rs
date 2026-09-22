use gateway_admin::{
    model::{
        MutationActor, MutationContext,
        client_keys::{
            ClientKeyBudgetPeriod, ClientKeyListQuery, ClientKeyPageSize, ClientKeySort,
            ClientKeySortField, DeleteClientKey, NewClientKey, ReplaceClientKeyIdentity,
            ResetClientKeyBudget, SetClientKeyEnabled, SortDirection, UpdateClientKey,
        },
    },
    ports::store::{AdminStoreErrorKind, ClientKeyStore},
};
use gateway_core::{
    account::OpaqueProviderData,
    policy::{ClientApiKeyId, RateLimits},
};
use gateway_store::postgres::PgAdminClientKeyStore;

use super::TestDatabase;

fn new_owned_key(id: &str) -> NewClientKey {
    NewClientKey {
        openai_client_profile_override: None,
        xai_client_profile_override: None,
        id: ClientApiKeyId::new(id).unwrap(),
        name: id.to_owned(),
        label: None,
        group_ids: Vec::new(),
        limits: RateLimits::unlimited(),
        budget: Default::default(),
        plaintext: format!("{id}-test-only-secret"),
    }
}

fn owned_key_query() -> ClientKeyListQuery {
    ClientKeyListQuery {
        cursor: None,
        page_size: ClientKeyPageSize::new(20).unwrap(),
        search: None,
        sort: ClientKeySort {
            field: ClientKeySortField::Name,
            direction: SortDirection::Asc,
        },
    }
}

fn update_owned_key(id: &str) -> UpdateClientKey {
    UpdateClientKey {
        openai_client_profile_override: None,
        xai_client_profile_override: None,
        id: ClientApiKeyId::new(id).unwrap(),
        name: format!("{id}-renamed"),
        label: Some("local QA".to_owned()),
        group_ids: Vec::new(),
        limits: RateLimits {
            max_concurrency: 2,
            requests_per_minute: 8,
        },
        daily_limit_usd: Some("1.5".parse().unwrap()),
        weekly_limit_usd: None,
    }
}

#[tokio::test]
async fn user_key_store_enforces_owner_on_every_operation_and_rolls_back_failures() {
    let Some(db) = TestDatabase::create("owned_key_store").await else {
        return;
    };
    sqlx::query(
        "insert into users (id,username,password_hash,role,created_at,updated_at)
        values ('user_a','owner-a','hash','user',now(),now()),
               ('user_b','owner-b','hash','user',now(),now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let store = PgAdminClientKeyStore::new(db.pool.clone());
    let (created_revision, created) = store
        .create_user_client_key("user_a", new_owned_key("key_owned_a"))
        .await
        .expect("user key creation must use implemented store capability");
    let (_, other) = store
        .create_user_client_key("user_b", new_owned_key("key_owned_b"))
        .await
        .unwrap();
    assert_ne!(created.id, other.id);
    assert!(created.groups.is_empty());
    assert!(created.provider_kinds.is_empty());
    let own = store
        .list_user_client_keys("user_a", owned_key_query())
        .await
        .unwrap();
    assert!(own.config_revision.get() >= created_revision.get());
    assert_eq!(own.total, 1);
    assert_eq!(own.items[0].id, created.id);
    assert!(
        store.get_client_key(&created.id).await.unwrap().is_none(),
        "generic ownerless read must not return an owned Key",
    );
    let usage_context = store
        .usage_budget_context(&created.id)
        .await
        .unwrap()
        .expect("verified Bearer usage lookup must resolve an owned Key");
    assert_eq!(usage_context.owner_user_id.as_deref(), Some("user_a"));
    assert!(usage_context.enabled);
    assert!(
        store
            .reveal_client_key(&created.id)
            .await
            .unwrap()
            .is_none(),
        "generic ownerless reveal must not return an owned Key",
    );
    assert_eq!(
        store
            .list_client_keys(owned_key_query())
            .await
            .unwrap()
            .total,
        0,
        "generic ownerless list must exclude owned Keys",
    );
    let secret = store
        .reveal_user_client_key("user_a", &created.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(secret.expose_for_response(), "key_owned_a-test-only-secret");
    assert!(
        store
            .reveal_user_client_key("user_b", &created.id)
            .await
            .unwrap()
            .is_none()
    );

    let context = MutationContext {
        actor: MutationActor::System,
        request_id: "owned-key-generic-isolation".to_owned(),
    };
    for error in [
        store
            .set_client_key_enabled(
                SetClientKeyEnabled {
                    id: created.id.clone(),
                    enabled: false,
                },
                &context,
            )
            .await
            .unwrap_err(),
        store
            .delete_client_key(
                DeleteClientKey {
                    id: created.id.clone(),
                },
                &context,
            )
            .await
            .unwrap_err(),
        store
            .reset_client_key_budget(
                ResetClientKeyBudget {
                    id: created.id.clone(),
                    period: ClientKeyBudgetPeriod::Daily,
                },
                &context,
            )
            .await
            .unwrap_err(),
        store
            .reset_user_client_key_budget(
                "user_b",
                ResetClientKeyBudget {
                    id: created.id.clone(),
                    period: ClientKeyBudgetPeriod::Daily,
                },
            )
            .await
            .unwrap_err(),
    ] {
        assert_eq!(error.kind(), AdminStoreErrorKind::NotFound);
    }

    let before: i64 = sqlx::query_scalar("select config_revision from runtime_settings where id=1")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let errors = [
        store
            .update_user_client_key("user_b", update_owned_key("key_owned_a"))
            .await
            .unwrap_err(),
        store
            .set_user_client_key_enabled(
                "user_b",
                SetClientKeyEnabled {
                    id: created.id.clone(),
                    enabled: false,
                },
            )
            .await
            .unwrap_err(),
        store
            .delete_user_client_key(
                "user_b",
                DeleteClientKey {
                    id: created.id.clone(),
                },
            )
            .await
            .unwrap_err(),
    ];
    for error in errors {
        assert_eq!(error.kind(), AdminStoreErrorKind::NotFound);
    }
    let after: i64 = sqlx::query_scalar("select config_revision from runtime_settings where id=1")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        before, after,
        "failed owner checks must roll back revision increments"
    );

    let (_, updated) = store
        .update_user_client_key("user_a", update_owned_key("key_owned_a"))
        .await
        .unwrap();
    assert_eq!(updated.name, "key_owned_a-renamed");
    assert_eq!(updated.limits.max_concurrency, 2);
    let (_, disabled) = store
        .set_user_client_key_enabled(
            "user_a",
            SetClientKeyEnabled {
                id: created.id.clone(),
                enabled: false,
            },
        )
        .await
        .unwrap();
    assert!(!disabled.enabled);
    let (_, enabled) = store
        .set_user_client_key_enabled(
            "user_a",
            SetClientKeyEnabled {
                id: created.id.clone(),
                enabled: true,
            },
        )
        .await
        .unwrap();
    assert!(enabled.enabled);
    store
        .delete_user_client_key(
            "user_a",
            DeleteClientKey {
                id: created.id.clone(),
            },
        )
        .await
        .unwrap();
    assert!(
        store
            .reveal_user_client_key("user_a", &created.id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store
            .list_user_client_keys("user_b", owned_key_query())
            .await
            .unwrap()
            .total,
        1
    );
    db.close().await;
}

#[tokio::test]
async fn usage_budget_context_suppresses_key_owned_by_disabled_user() {
    let Some(db) = TestDatabase::create("owned_key_usage_disabled_owner").await else {
        return;
    };
    sqlx::query(
        "insert into users (id,username,password_hash,role,enabled,created_at,updated_at)
         values ('user_usage_owner','usage-owner','hash','user',true,now(),now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let store = PgAdminClientKeyStore::new(db.pool.clone());
    let (_, key) = store
        .create_user_client_key("user_usage_owner", new_owned_key("key_usage_owner"))
        .await
        .unwrap();
    assert!(
        store
            .usage_budget_context(&key.id)
            .await
            .unwrap()
            .unwrap()
            .enabled
    );

    sqlx::query("update users set enabled=false where id='user_usage_owner'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(
        !store
            .usage_budget_context(&key.id)
            .await
            .unwrap()
            .unwrap()
            .enabled
    );
    db.close().await;
}

#[tokio::test]
async fn owned_key_projects_user_groups_and_preserves_admin_profile_configuration() {
    let Some(db) = TestDatabase::create("owned_key_groups").await else {
        return;
    };
    sqlx::query(
        "insert into users (id,username,password_hash,role,created_at,updated_at)
        values ('user_group_owner','group-owner','hash','user',now(),now()),
               ('user_b','other-owner','hash','user',now(),now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "insert into provider_accounts
        (id,provider_kind,name,authentication_kind,provider_credentials_json,has_refresh_token,
         credential_observed_at,created_at,updated_at)
        values ('acct_owned_test','openai','Test','api_key','{}',false,now(),now(),now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "insert into account_groups (id,name,color,created_at,updated_at)
        values ('grp_00000000000000000000000000000009','Owner Group','#2563EBFF',now(),now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "insert into account_group_accounts (account_group_id,provider_account_id,created_at)
        values ('grp_00000000000000000000000000000009','acct_owned_test',now())",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let store = PgAdminClientKeyStore::new(db.pool.clone());
    let (_, record) = store
        .create_user_client_key("user_group_owner", new_owned_key("key_group_owned"))
        .await
        .unwrap();
    assert!(
        record.provider_kinds.is_empty(),
        "owned empty group must not expose global providers"
    );
    let context = MutationContext {
        actor: MutationActor::System,
        request_id: "owned-key-legacy-qa".to_owned(),
    };
    let (_, legacy) = store
        .create_client_key(new_owned_key("key_group_legacy"), &context)
        .await
        .unwrap();
    assert_eq!(
        legacy.provider_kinds.len(),
        1,
        "ownerless empty group keeps upstream all-provider semantics"
    );

    sqlx::query(
        "insert into user_account_groups (user_id,account_group_id)
        values ('user_group_owner','grp_00000000000000000000000000000009')",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    let page = store
        .list_user_client_keys("user_group_owner", owned_key_query())
        .await
        .unwrap();
    assert_eq!(page.items[0].groups.len(), 1);
    assert_eq!(page.items[0].provider_kinds[0].as_str(), "openai");
    let detail = store
        .reveal_user_client_key("user_group_owner", &record.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(detail.record.groups, page.items[0].groups);

    let key_profile = OpaqueProviderData::new(
        serde_json::json!({"versionMode":"latest"})
            .as_object()
            .unwrap()
            .clone(),
    );
    let (_, identity_record) = store
        .replace_user_client_key_identity(
            "user_group_owner",
            ReplaceClientKeyIdentity {
                id: record.id.clone(),
                openai_client_profile_override: Some(key_profile.clone()),
                xai_client_profile_override: None,
            },
            &context,
        )
        .await
        .unwrap();
    assert_eq!(
        identity_record.openai_client_profile_override,
        Some(key_profile.clone())
    );
    assert_eq!(
        store
            .replace_user_client_key_identity(
                "user_b",
                ReplaceClientKeyIdentity {
                    id: record.id.clone(),
                    openai_client_profile_override: Some(key_profile.clone()),
                    xai_client_profile_override: None,
                },
                &context,
            )
            .await
            .unwrap_err()
            .kind(),
        AdminStoreErrorKind::NotFound,
        "cross-user identity update must not find another owner's Key",
    );
    store
        .update_user_client_key("user_group_owner", update_owned_key("key_group_owned"))
        .await
        .unwrap();
    let profile: serde_json::Value = sqlx::query_scalar(
        "select provider_request_profiles_json from client_api_keys where id='key_group_owned'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        profile,
        serde_json::json!({"openai":{"versionMode":"latest"}}),
        "ordinary owned-Key updates must preserve administrator identity overrides",
    );

    sqlx::query(
        "update users set provider_request_profiles_json = '{\"openai\":{\"versionMode\":\"fixed\"},\"xai\":{\"versionMode\":\"latest\"}}'::jsonb where id='user_group_owner'",
    )
    .execute(&db.pool)
    .await
    .unwrap();
    use gateway_store::postgres::{PgRuntimeSnapshotRepository, RuntimeSnapshotRepository};
    let snapshot = PgRuntimeSnapshotRepository::new(db.pool.clone())
        .load_runtime_snapshot()
        .await
        .unwrap();
    let key_snapshot = snapshot
        .client_api_keys
        .iter()
        .find(|key| key.id.as_str() == "key_group_owned")
        .unwrap();
    assert_eq!(
        key_snapshot
            .request_profiles
            .get(&gateway_core::routing::ProviderKind::new("openai").unwrap()),
        Some(&key_profile),
        "Key OpenAI override must win over the User default",
    );
    let xai_profile = OpaqueProviderData::new(
        serde_json::json!({"versionMode":"latest"})
            .as_object()
            .unwrap()
            .clone(),
    );
    assert_eq!(
        key_snapshot
            .request_profiles
            .get(&gateway_core::routing::ProviderKind::new("xai").unwrap()),
        Some(&xai_profile),
        "providers without a Key override must inherit the User default",
    );

    assert_eq!(
        store
            .update_client_key(update_owned_key("key_group_owned"), &context)
            .await
            .unwrap_err()
            .kind(),
        AdminStoreErrorKind::NotFound,
        "generic ownerless writes must never mutate an owned Key",
    );
    let mut forbidden = update_owned_key("key_group_owned");
    forbidden.openai_client_profile_override = Some(None);
    assert_eq!(
        store
            .update_user_client_key("user_group_owner", forbidden)
            .await
            .unwrap_err()
            .kind(),
        AdminStoreErrorKind::Invalid
    );

    sqlx::query("delete from user_account_groups where user_id='user_group_owner'")
        .execute(&db.pool)
        .await
        .unwrap();
    let without_groups = store
        .list_user_client_keys("user_group_owner", owned_key_query())
        .await
        .unwrap();
    assert!(without_groups.items[0].groups.is_empty());
    assert!(without_groups.items[0].provider_kinds.is_empty());
    sqlx::query("update users set enabled=false where id='user_group_owner'")
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(
        store
            .create_user_client_key("user_group_owner", new_owned_key("key_disabled_owner"))
            .await
            .is_err()
    );
    db.close().await;
}

#[tokio::test]
async fn owned_keys_are_isolated_and_ownerless_legacy_keys_remain_valid() {
    let Some(db) = TestDatabase::create("owned_keys").await else {
        return;
    };
    sqlx::query(
        "insert into users (id, username, password_hash, role, created_at, updated_at)
         values ('user_key_owner', 'key-owner', 'hash', 'user', now(), now())",
    )
    .execute(&db.pool)
    .await
    .expect("insert key owner");
    sqlx::query(
        "insert into client_api_keys
         (id, name, key, owner_user_id, created_at, updated_at)
         values ('key_owned', 'owned', 'sk_' || repeat('a', 43), 'user_key_owner', now(), now()),
                ('key_legacy', 'legacy', 'sk_' || repeat('b', 43), null, now(), now())",
    )
    .execute(&db.pool)
    .await
    .expect("insert owned and ownerless keys");
    let owned: Option<String> =
        sqlx::query_scalar("select owner_user_id from client_api_keys where id = 'key_owned'")
            .fetch_one(&db.pool)
            .await
            .expect("read owned key");
    assert_eq!(owned.as_deref(), Some("user_key_owner"));
    sqlx::query("delete from users where id = 'user_key_owner'")
        .execute(&db.pool)
        .await
        .expect("delete owner");
    let remaining: Vec<String> = sqlx::query_scalar("select id from client_api_keys order by id")
        .fetch_all(&db.pool)
        .await
        .expect("read remaining keys");
    assert_eq!(remaining, vec!["key_legacy"]);
    db.close().await;
}
