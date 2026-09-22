use gateway_store::{
    ConflictKind, StoreError,
    postgres::{PgIdentityRepository, StoredUser, StoredUserUpdate},
};

use super::TestDatabase;

#[tokio::test]
async fn user_identity_migration_and_repository_should_enforce_bootstrap_and_unique_username() {
    let Some(database) = TestDatabase::create("identity").await else {
        return;
    };
    let repository = PgIdentityRepository::new(database.pool.clone());

    let settings = repository.auth_settings().await.expect("auth settings");
    assert!(!settings.turnstile_enabled);
    assert_eq!(settings.turnstile_site_key, None);
    assert_eq!(settings.turnstile_secret_key, None);

    assert!(
        repository
            .ensure_default_admin("admin", "$argon2id$test-bootstrap")
            .await
            .expect("bootstrap admin")
    );
    assert!(
        !repository
            .ensure_default_admin("admin", "$argon2id$ignored")
            .await
            .expect("repeat bootstrap")
    );
    let admin = repository
        .user_by_id("admin")
        .await
        .expect("load admin")
        .expect("admin exists");
    assert_eq!(admin.username, "admin");
    let debug = format!("{admin:?}");
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains(&admin.password_hash));
    assert_eq!(admin.role, "admin");
    assert_eq!(admin.max_concurrency, None);
    assert_eq!(admin.requests_per_minute, None);
    assert!(admin.enabled);
    let legacy_admin =
        sqlx::query_scalar::<_, i64>("select count(*) from admin_users where id = 'admin'")
            .fetch_one(&database.pool)
            .await
            .expect("legacy admin row");
    assert_eq!(legacy_admin, 1);

    let now = chrono::Utc::now();
    let alice = StoredUser {
        id: "user_alice".to_owned(),
        username: "alice".to_owned(),
        password_hash: "$argon2id$alice".to_owned(),
        role: "user".to_owned(),
        enabled: true,
        max_concurrency: None,
        requests_per_minute: None,
        session_version: 1,
        created_at: now,
        updated_at: now,
    };
    sqlx::query(
        "insert into users (id, username, password_hash, role, enabled, created_at, updated_at)
         values ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(&alice.id)
    .bind(&alice.username)
    .bind(&alice.password_hash)
    .bind(&alice.role)
    .bind(alice.enabled)
    .bind(alice.created_at)
    .bind(alice.updated_at)
    .execute(&database.pool)
    .await
    .expect("insert first username");
    let regular = repository.user_by_id(&alice.id).await.unwrap().unwrap();
    assert_eq!(regular.max_concurrency, Some(0));
    assert_eq!(regular.requests_per_minute, Some(0));
    let duplicate = sqlx::query(
        "insert into users (id, username, password_hash, role, enabled, created_at, updated_at)
         values ('user_alice_2', 'alice', '$argon2id$alice2', 'user', true, now(), now())",
    )
    .execute(&database.pool)
    .await;
    assert!(
        duplicate.is_err(),
        "username uniqueness must be database-enforced"
    );
    assert!(
        repository
            .update_username(
                &alice.id,
                "alice-stale",
                u64::try_from(regular.session_version + 1).unwrap(),
            )
            .await
            .expect("stale username CAS")
            .is_none(),
        "a stale or revoked session version must not rename the account",
    );
    assert_eq!(
        repository
            .user_by_id(&alice.id)
            .await
            .unwrap()
            .unwrap()
            .username,
        "alice"
    );
    let renamed = repository
        .update_username(
            &alice.id,
            "alice-renamed",
            u64::try_from(regular.session_version).unwrap(),
        )
        .await
        .expect("current username CAS")
        .expect("current session version may rename");
    assert_eq!(renamed.username, "alice-renamed");

    assert!(
        repository
            .update_password_hash_if_matches(&alice.id, &alice.password_hash, "$argon2id$alice-new")
            .await
            .expect("atomic self-service password update")
    );
    assert!(
        !repository
            .update_password_hash_if_matches(&alice.id, &alice.password_hash, "$argon2id$stale")
            .await
            .expect("stale self-service password update")
    );
    assert_eq!(
        repository
            .user_by_id(&alice.id)
            .await
            .expect("reload changed user")
            .expect("changed user exists")
            .session_version,
        2
    );

    let bob = StoredUser {
        id: "user_bob".to_owned(),
        username: "bob".to_owned(),
        max_concurrency: Some(0),
        requests_per_minute: Some(5),
        ..alice.clone()
    };
    repository
        .create_user(&bob, identity_audit("create", &bob.id))
        .await
        .expect("create with finite zero");
    let initial: (Option<i64>, Option<i64>) = sqlx::query_as(
        "select max_concurrency_override,requests_per_minute_override from users where id=$1",
    )
    .bind(&bob.id)
    .fetch_one(&database.pool)
    .await
    .unwrap();
    assert_eq!(initial, (Some(0), Some(5)));
    let audit = identity_audit("update", &bob.id);
    let audit_id = audit.id.clone();
    let (revision, updated) = repository
        .update_user(
            StoredUserUpdate {
                id: &bob.id,
                username: None,
                role: None,
                enabled: None,
                max_concurrency: Some(Some(3)),
                requests_per_minute: Some(Some(0)),
            },
            audit,
        )
        .await
        .expect("update paired overrides");
    assert_eq!(updated.role, "user");
    assert_eq!(updated.max_concurrency, Some(3));
    assert_eq!(updated.requests_per_minute, Some(0));
    let recorded: Option<i64> =
        sqlx::query_scalar("select config_revision from admin_audit_events where id=$1")
            .bind(audit_id)
            .fetch_one(&database.pool)
            .await
            .unwrap();
    assert_eq!(recorded, Some(i64::try_from(revision.get()).unwrap()));

    let before_role_version = updated.session_version;
    let (_revision, promoted) = repository
        .update_user(
            StoredUserUpdate {
                id: &bob.id,
                username: None,
                role: Some("admin"),
                enabled: None,
                max_concurrency: None,
                requests_per_minute: None,
            },
            identity_audit("promote", &bob.id),
        )
        .await
        .expect("promote user to administrator");
    assert_eq!(promoted.role, "admin");
    assert_eq!(promoted.session_version, before_role_version + 1);
    let promoted_admin_projection =
        sqlx::query_scalar::<_, i64>("select count(*) from admin_users where id = $1")
            .bind(&bob.id)
            .fetch_one(&database.pool)
            .await
            .expect("promoted admin projection");
    assert_eq!(promoted_admin_projection, 1);

    database.close().await;
}

#[tokio::test]
async fn last_enabled_admin_is_enforced_transactionally() {
    let Some(database) = TestDatabase::create("last_enabled_admin").await else {
        return;
    };
    let repository = PgIdentityRepository::new(database.pool.clone());
    repository
        .ensure_default_admin("only_admin", "$argon2id$test-bootstrap")
        .await
        .expect("bootstrap only admin");

    for (action, update) in [
        (
            "demote",
            StoredUserUpdate {
                id: "only_admin",
                username: None,
                role: Some("user"),
                enabled: None,
                max_concurrency: None,
                requests_per_minute: None,
            },
        ),
        (
            "disable",
            StoredUserUpdate {
                id: "only_admin",
                username: None,
                role: None,
                enabled: Some(false),
                max_concurrency: None,
                requests_per_minute: None,
            },
        ),
    ] {
        let error = repository
            .update_user(update, identity_audit(action, "only_admin"))
            .await
            .expect_err("last enabled administrator transition must fail");
        assert!(matches!(
            error,
            StoreError::Conflict {
                kind: ConflictKind::InvalidTransition,
                ..
            }
        ));
    }

    let delete_error = repository
        .delete_user(
            "only_admin",
            identity_audit("delete-last-admin", "only_admin"),
        )
        .await
        .expect_err("last enabled administrator delete must fail");
    assert!(matches!(
        delete_error,
        StoreError::Conflict {
            kind: ConflictKind::InvalidTransition,
            ..
        }
    ));

    let now = chrono::Utc::now();
    let backup = StoredUser {
        id: "backup_admin".to_owned(),
        username: "backup-admin".to_owned(),
        password_hash: "$argon2id$backup".to_owned(),
        role: "admin".to_owned(),
        enabled: true,
        max_concurrency: Some(0),
        requests_per_minute: Some(0),
        session_version: 1,
        created_at: now,
        updated_at: now,
    };
    repository
        .create_user(&backup, identity_audit("create-backup-admin", &backup.id))
        .await
        .expect("create backup administrator");
    repository
        .delete_user(
            "only_admin",
            identity_audit("delete-with-backup", "only_admin"),
        )
        .await
        .expect("delete administrator when backup remains");
    assert!(
        repository
            .user_by_id("only_admin")
            .await
            .expect("load deleted administrator")
            .is_none()
    );
    assert!(
        repository
            .user_by_id("backup_admin")
            .await
            .expect("load backup administrator")
            .is_some()
    );

    database.close().await;
}

fn identity_audit(action: &str, id: &str) -> gateway_store::postgres::AdminAuditEvent {
    use gateway_store::postgres::{AdminAuditActorKind, AdminAuditEvent};
    AdminAuditEvent {
        id: format!("audit_{}", uuid::Uuid::new_v4().simple()),
        actor_kind: AdminAuditActorKind::System,
        actor_admin_user_id: None,
        actor_ref: "test".to_owned(),
        admin_request_id: None,
        action: action.to_owned(),
        entity_kind: "user".to_owned(),
        entity_ref: id.to_owned(),
        config_revision: None,
        changed_fields: vec!["limits".to_owned()],
        created_at: chrono::Utc::now(),
    }
}

#[tokio::test]
async fn deleting_user_clears_owned_state_but_keeps_billing_history() {
    let Some(database) = TestDatabase::create("identity_delete").await else {
        return;
    };
    let identity = PgIdentityRepository::new(database.pool.clone());
    sqlx::query(
        "insert into users(id,username,password_hash,role,enabled,created_at,updated_at)
         values('delete_user','delete-user','test-hash','user',true,now(),now())",
    )
    .execute(&database.pool)
    .await
    .expect("seed delete user");
    sqlx::query(
        "insert into client_api_keys(id,name,key,enabled,created_at,updated_at,owner_user_id)
         values('delete_key','delete-key','sk_1234567890123456789012345678901234567890123',true,now(),now(),'delete_user')",
    )
    .execute(&database.pool)
    .await
    .expect("seed owned key");
    sqlx::query(
        "insert into user_budget_windows(user_id,daily_start,daily_end,weekly_start,weekly_end,monthly_start,monthly_end,daily_used_usd)
         values('delete_user',now()-interval '1 hour',now()+interval '23 hours',now()-interval '1 hour',now()+interval '6 days',date_trunc('month',now()),date_trunc('month',now())+interval '1 month',1)",
    )
    .execute(&database.pool)
    .await
    .expect("seed budget window");
    let plan_id: String =
        sqlx::query_scalar("select id from subscription_plans where is_base limit 1")
            .fetch_one(&database.pool)
            .await
            .expect("base plan");
    sqlx::query(
        "insert into user_subscriptions(id,user_id,plan_id,status,starts_at,expires_at,created_at,updated_at)
         values('sub_00000000000000000000000000000001','delete_user',$1,'active',now()-interval '1 hour',now()+interval '1 day',now(),now())",
    )
    .bind(&plan_id)
    .execute(&database.pool)
    .await
    .expect("seed historical subscription");
    sqlx::query(
        "insert into user_charge_events(request_id,user_id,subscription_id, billing_group_ref,base_amount_usd,downstream_rate_multiplier,billed_amount_usd,completed_at)
         values('delete_request','delete_user','sub_00000000000000000000000000000001','historical-group',1,1,1,now())",
    )
    .execute(&database.pool)
    .await
    .expect("seed historical charge");
    identity
        .delete_user("delete_user", identity_audit("user.delete", "delete_user"))
        .await
        .expect("delete user");
    assert!(!sqlx::query_scalar::<_, bool>(
        "select exists(select 1 from users where id='delete_user')",
    )
    .fetch_one(&database.pool)
    .await
    .expect("check deleted user"));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("select count(*) from client_api_keys where id='delete_key'")
            .fetch_one(&database.pool)
            .await
            .expect("check owned key"),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "select count(*) from user_budget_windows where user_id='delete_user'",
        )
        .fetch_one(&database.pool)
        .await
        .expect("check budget window"),
        0
    );
    sqlx::query("delete from user_subscriptions where id='sub_00000000000000000000000000000001'")
        .execute(&database.pool)
        .await
        .expect("delete historical subscription");
    let historical: (Option<String>, Option<String>, String, Option<String>) = sqlx::query_as(
        "select user_id,subscription_id,user_ref,subscription_ref from user_charge_events where request_id='delete_request'",
    )
    .fetch_one(&database.pool)
    .await
    .expect("load historical charge");
    assert_eq!(
        historical,
        (
            None,
            None,
            "delete_user".to_owned(),
            Some("sub_00000000000000000000000000000001".to_owned())
        )
    );
    database.close().await;
}

#[tokio::test]
async fn newly_created_admin_and_existing_admin_can_write_login_audit() {
    use gateway_store::postgres::{
        AdminAuditActorKind, AdminSecurityAuditRepository, PgAdminSecurityAuditRepository,
    };
    let Some(database) = TestDatabase::create("new_admin_audit").await else {
        return;
    };
    let identity = PgIdentityRepository::new(database.pool.clone());
    let security = PgAdminSecurityAuditRepository::new(database.pool.clone());
    let now = chrono::Utc::now();
    let admin = StoredUser {
        id: "new_admin".to_owned(),
        username: "new-admin".to_owned(),
        password_hash: "$argon2id$test-only".to_owned(),
        role: "admin".to_owned(),
        enabled: true,
        max_concurrency: None,
        requests_per_minute: None,
        session_version: 1,
        created_at: now,
        updated_at: now,
    };
    identity
        .create_user(&admin, identity_audit("create", &admin.id))
        .await
        .unwrap();
    let projected: bool =
        sqlx::query_scalar("select exists(select 1 from admin_users where id=$1)")
            .bind(&admin.id)
            .fetch_one(&database.pool)
            .await
            .unwrap();
    assert!(projected);
    let mut login = identity_audit("admin.login", &admin.id);
    login.actor_kind = AdminAuditActorKind::AdminSession;
    login.actor_admin_user_id = Some(admin.id.clone());
    login.actor_ref = format!("admin:{}", admin.id);
    security
        .append_admin_audit_event(login)
        .await
        .expect("new Admin login audit FK");

    // 模拟修复前已存在但缺少审计兼容行的 Admin，不通过删除已有审计身份制造场景。
    sqlx::query(
        "insert into users(id,username,password_hash,role,enabled,created_at,updated_at)
        values('existing_admin','existing-admin','test-hash','admin',true,now(),now())",
    )
    .execute(&database.pool)
    .await
    .unwrap();
    identity
        .ensure_admin_audit_identity("existing_admin")
        .await
        .unwrap();
    identity
        .ensure_admin_audit_identity("existing_admin")
        .await
        .unwrap();
    let mut login = identity_audit("admin.login", "existing_admin");
    login.actor_kind = AdminAuditActorKind::AdminSession;
    login.actor_admin_user_id = Some("existing_admin".to_owned());
    login.actor_ref = "admin:existing_admin".to_owned();
    security
        .append_admin_audit_event(login)
        .await
        .expect("existing Admin login audit FK");
    let count: i64 =
        sqlx::query_scalar("select count(*) from admin_users where id='existing_admin'")
            .fetch_one(&database.pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        identity
            .user_by_id("existing_admin")
            .await
            .unwrap()
            .unwrap()
            .password_hash,
        "test-hash"
    );
    database.close().await;
}

#[tokio::test]
async fn administrator_profile_updates_are_user_facts_inherited_by_existing_and_new_keys() {
    use gateway_admin::model::users::UserKeyIdentity;
    use gateway_core::{account::OpaqueProviderData, identity::ProviderKind};
    use gateway_store::postgres::{PgRuntimeSnapshotRepository, RuntimeSnapshotRepository};
    let Some(database) = TestDatabase::create("identity_key_profiles").await else {
        return;
    };
    let repository = PgIdentityRepository::new(database.pool.clone());
    repository
        .ensure_default_admin("identity_admin", "test-hash")
        .await
        .unwrap();
    let global = repository.key_identity("identity_admin").await.unwrap();
    assert_eq!(global, UserKeyIdentity::default());
    for id in ["identity_key_a", "identity_key_b"] {
        sqlx::query("insert into client_api_keys(id,owner_user_id,name,key,enabled,created_at,updated_at) values($1,'identity_admin',$1,$1,true,now(),now())")
            .bind(id).execute(&database.pool).await.unwrap();
    }
    let profile = OpaqueProviderData::new(
        serde_json::json!({"client":"cli","platform":"linux","versionMode":"latest"})
            .as_object()
            .unwrap()
            .clone(),
    );
    repository
        .replace_key_identity(
            "identity_admin",
            UserKeyIdentity {
                openai: Some(profile.clone()),
                xai: None,
            },
            identity_audit("user.key_identity.update", "identity_admin"),
        )
        .await
        .unwrap();
    let snapshot = PgRuntimeSnapshotRepository::new(database.pool.clone())
        .load_runtime_snapshot()
        .await
        .unwrap();
    let kind = ProviderKind::new("openai").unwrap();
    assert_eq!(snapshot.client_api_keys.len(), 2);
    for key in &snapshot.client_api_keys {
        assert_eq!(key.request_profiles.get(&kind), Some(&profile));
    }
    let copied: i64 = sqlx::query_scalar(
        "select count(*) from client_api_keys where provider_request_profiles_json <> '{}'::jsonb",
    )
    .fetch_one(&database.pool)
    .await
    .unwrap();
    assert_eq!(copied, 0, "never copy identity to individual keys");
    repository
        .replace_key_identity(
            "identity_admin",
            UserKeyIdentity::default(),
            identity_audit("user.key_identity.update", "identity_admin"),
        )
        .await
        .unwrap();
    let snapshot = PgRuntimeSnapshotRepository::new(database.pool.clone())
        .load_runtime_snapshot()
        .await
        .unwrap();
    assert!(
        snapshot
            .client_api_keys
            .iter()
            .all(|key| key.request_profiles.is_empty())
    );
    repository
        .replace_key_identity(
            "identity_admin",
            UserKeyIdentity {
                openai: Some(OpaqueProviderData::new(serde_json::Map::new())),
                xai: None,
            },
            identity_audit("user.key_identity.update", "identity_admin"),
        )
        .await
        .unwrap();
    assert_eq!(
        repository.key_identity("identity_admin").await.unwrap(),
        UserKeyIdentity::default(),
        "empty profile objects normalize to inherit/global",
    );
    let stored_profiles: serde_json::Value = sqlx::query_scalar(
        "select provider_request_profiles_json from users where id='identity_admin'",
    )
    .fetch_one(&database.pool)
    .await
    .unwrap();
    assert_eq!(stored_profiles, serde_json::json!({}));
    let audited: i64 = sqlx::query_scalar(
        "select count(*) from admin_audit_events where action='user.key_identity.update'",
    )
    .fetch_one(&database.pool)
    .await
    .unwrap();
    assert_eq!(audited, 3);
    database.close().await;
}
