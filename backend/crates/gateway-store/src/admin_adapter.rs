//! Admin 认证与设置 adapter。

use super::*;
use gateway_core::policy::UserRateLimits;

pub(crate) struct AuthStoreAdapter {
    pub(crate) security: postgres::PgAdminSecurityAuditRepository,
    pub(crate) identity: postgres::PgIdentityRepository,
    pub(crate) settings: postgres::PgRuntimeSettingsRepository,
    pub(crate) state: redis::RedisAuthStateRepository,
    pub(crate) keys: postgres::PgAdminClientKeyStore,
}

pub(crate) struct AdminSettingsStoreAdapter {
    pub(crate) control_plane: postgres::PgControlPlaneRepository,
}

#[async_trait::async_trait]
impl SettingsStore for AdminSettingsStoreAdapter {
    async fn load_pricing(&self) -> AdminStoreResult<gateway_admin::model::pricing::StoredPricing> {
        self.control_plane
            .load_pricing()
            .await
            .map_err(|error| admin_store_error("model pricing", error))
    }

    async fn sync_pricing(
        &self,
        changes: gateway_admin::model::pricing::PricingSyncChanges,
        context: &MutationContext,
    ) -> AdminStoreResult<gateway_admin::model::Revision> {
        let audit = mutation_audit(
            context,
            "pricing.sync",
            "model_pricing",
            "models.dev",
            vec!["synced".to_owned()],
        );
        let revision = self
            .control_plane
            .sync_pricing(changes, audit)
            .await
            .map_err(|error| admin_store_error("model pricing sync", error))?;
        admin_revision(revision)
    }

    async fn update_pricing(
        &self,
        command: gateway_admin::model::pricing::UpdatePricing,
        context: &MutationContext,
    ) -> AdminStoreResult<gateway_admin::model::Revision> {
        let audit = mutation_audit(
            context,
            "pricing.update",
            "model_pricing",
            &command.provider,
            command.models.clone(),
        );
        let revision = self
            .control_plane
            .update_pricing(command, audit)
            .await
            .map_err(|error| admin_store_error("model pricing", error))?;
        admin_revision(revision)
    }

    async fn load_runtime_settings(&self) -> AdminStoreResult<AdminRuntimeSettings> {
        let snapshot = postgres::ControlPlaneRepository::load_control_plane(&self.control_plane)
            .await
            .map_err(|error| admin_store_error("runtime settings", error))?;
        admin_runtime_settings(snapshot.settings)
    }

    async fn admin_api_key_exists(&self) -> AdminStoreResult<bool> {
        postgres::ControlPlaneRepository::load_control_plane(&self.control_plane)
            .await
            .map(|snapshot| snapshot.settings.admin_api_key.is_some())
            .map_err(|error| admin_store_error("admin API key", error))
    }

    async fn replace_runtime_settings(
        &self,
        command: ReplaceRuntimeSettings,
        context: &MutationContext,
    ) -> AdminStoreResult<AdminRuntimeSettings> {
        let current = postgres::ControlPlaneRepository::load_control_plane(&self.control_plane)
            .await
            .map_err(|error| admin_store_error("runtime settings", error))?;
        let replacement = postgres::ControlPlaneReplacement {
            settings: postgres::RuntimeSettingsUpdate {
                openai_client_profile: command.openai_client_profile,
                xai_client_profile: command.xai_client_profile,
                admin_api_key: current.settings.admin_api_key,
                refresh_margin_seconds: command.refresh_margin_seconds,
                refresh_concurrency: command.refresh_concurrency,
                max_concurrent_per_account: command.max_concurrent_per_account,
                request_location_enabled: command.request_location_enabled,
                request_location: command.request_location,
                request_interval_ms: command.request_interval_ms,
                max_waiting_per_key: command.max_waiting_per_key,
                max_waiting_per_account: command.max_waiting_per_account,
                concurrency_wait_timeout_seconds: command.concurrency_wait_timeout_seconds,
                responses_max_decompressed_body_bytes: command
                    .responses_max_decompressed_body_bytes,
                rotation_strategy: command.rotation_strategy.as_str().to_owned(),
                model_mappings: store_model_mappings(command.model_mappings),
                min_codex_desktop_version: command.min_codex_desktop_version,
                min_codex_cli_version: command.min_codex_cli_version,
                usage_retention_days: command.usage_retention_days,
                ops_event_retention_days: command.ops_event_retention_days,
                audit_retention_days: command.audit_retention_days,
                account_auto_freeze_enabled: command.account_auto_freeze_enabled,
                account_auto_freeze_threshold: command.account_auto_freeze_threshold,
                account_auto_freeze_window_seconds: command.account_auto_freeze_window_seconds,
                account_auto_freeze_duration_seconds: command.account_auto_freeze_duration_seconds,
                account_auto_freeze_probe_enabled: command.account_auto_freeze_probe_enabled,
                account_auto_freeze_probe_model: command.account_auto_freeze_probe_model,
                account_auto_freeze_adaptive_concurrency: command
                    .account_auto_freeze_adaptive_concurrency,
            },
            audit: mutation_audit(
                context,
                "settings.replace",
                "runtime_settings",
                "1",
                vec![
                    "provider_request_profiles_json".to_owned(),
                    "request_location_enabled".to_owned(),
                    "request_location_json".to_owned(),
                    "model_mappings_json".to_owned(),
                    "refresh_margin_seconds".to_owned(),
                    "refresh_concurrency".to_owned(),
                    "max_concurrent_per_account".to_owned(),
                    "request_interval_ms".to_owned(),
                    "max_waiting_per_key".to_owned(),
                    "max_waiting_per_account".to_owned(),
                    "concurrency_wait_timeout_seconds".to_owned(),
                    "responses_max_decompressed_body_bytes".to_owned(),
                    "rotation_strategy".to_owned(),
                    "min_codex_desktop_version".to_owned(),
                    "min_codex_cli_version".to_owned(),
                    "retention".to_owned(),
                    "account_auto_freeze".to_owned(),
                ],
            ),
        };
        let snapshot = postgres::ControlPlaneRepository::replace_control_plane(
            &self.control_plane,
            replacement,
        )
        .await
        .map_err(|error| admin_store_error("runtime settings", error))?;
        admin_runtime_settings(snapshot.settings)
    }

    async fn replace_admin_api_key(
        &self,
        key: AdminApiKey,
        context: &MutationContext,
    ) -> AdminStoreResult<AdminApiKeyMutation> {
        self.replace_admin_api_key_value(Some(key.expose_for_auth().to_owned()), context)
            .await
    }

    async fn delete_admin_api_key(
        &self,
        context: &MutationContext,
    ) -> AdminStoreResult<AdminApiKeyMutation> {
        self.replace_admin_api_key_value(None, context).await
    }
}

impl AdminSettingsStoreAdapter {
    async fn replace_admin_api_key_value(
        &self,
        admin_api_key: Option<String>,
        context: &MutationContext,
    ) -> AdminStoreResult<AdminApiKeyMutation> {
        let exists = admin_api_key.is_some();
        let revision = postgres::ControlPlaneRepository::replace_admin_api_key(
            &self.control_plane,
            admin_api_key,
            mutation_audit(
                context,
                if exists {
                    "admin_api_key.replace"
                } else {
                    "admin_api_key.delete"
                },
                "runtime_settings",
                "1",
                vec!["admin_api_key".to_owned()],
            ),
        )
        .await
        .map_err(|error| admin_store_error("admin API key", error))?;
        Ok(AdminApiKeyMutation {
            config_revision: admin_revision(revision)?,
            exists,
        })
    }
}

pub(crate) fn admin_runtime_settings(
    settings: postgres::RuntimeSettings,
) -> AdminStoreResult<AdminRuntimeSettings> {
    let rotation_strategy = AdminRotationStrategy::parse(settings.rotation_strategy.as_str())
        .ok_or_else(|| {
            AdminStoreError::new(
                AdminStoreErrorKind::Invalid,
                "runtime settings",
                "rotation strategy is invalid",
            )
        })?;
    let model_mappings = settings
        .model_mappings
        .into_iter()
        .map(|(public, upstream)| {
            let public = gateway_core::routing::PublicModelId::new(public).map_err(|_| {
                AdminStoreError::new(
                    AdminStoreErrorKind::Invalid,
                    "runtime settings",
                    "public model mapping is invalid",
                )
            })?;
            let upstream = gateway_core::routing::UpstreamModelId::new(upstream).map_err(|_| {
                AdminStoreError::new(
                    AdminStoreErrorKind::Invalid,
                    "runtime settings",
                    "upstream model mapping is invalid",
                )
            })?;
            Ok((public, upstream))
        })
        .collect::<AdminStoreResult<ModelMappings>>()?;
    Ok(AdminRuntimeSettings {
        openai_client_profile: settings.openai_client_profile,
        xai_client_profile: settings.xai_client_profile,
        config_revision: admin_revision(settings.config_revision)?,
        request_location_enabled: settings.request_location_enabled,
        request_location: settings.request_location,
        model_mappings,
        refresh_margin_seconds: settings.refresh_margin_seconds,
        refresh_concurrency: settings.refresh_concurrency,
        max_concurrent_per_account: settings.max_concurrent_per_account,
        request_interval_ms: settings.request_interval_ms,
        max_waiting_per_key: settings.max_waiting_per_key,
        max_waiting_per_account: settings.max_waiting_per_account,
        concurrency_wait_timeout_seconds: settings.concurrency_wait_timeout_seconds,
        responses_max_decompressed_body_bytes: settings.responses_max_decompressed_body_bytes,
        rotation_strategy,
        min_codex_desktop_version: settings.min_codex_desktop_version,
        min_codex_cli_version: settings.min_codex_cli_version,
        usage_retention_days: settings.usage_retention_days,
        ops_event_retention_days: settings.ops_event_retention_days,
        audit_retention_days: settings.audit_retention_days,
        account_auto_freeze_enabled: settings.account_auto_freeze_enabled,
        account_auto_freeze_threshold: settings.account_auto_freeze_threshold,
        account_auto_freeze_window_seconds: settings.account_auto_freeze_window_seconds,
        account_auto_freeze_duration_seconds: settings.account_auto_freeze_duration_seconds,
        account_auto_freeze_probe_enabled: settings.account_auto_freeze_probe_enabled,
        account_auto_freeze_probe_model: settings.account_auto_freeze_probe_model,
        account_auto_freeze_adaptive_concurrency: settings.account_auto_freeze_adaptive_concurrency,
        updated_at: settings.updated_at,
    })
}

pub(crate) fn store_model_mappings(
    mappings: ModelMappings,
) -> std::collections::BTreeMap<String, String> {
    mappings
        .into_iter()
        .map(|(public, upstream)| (public.as_str().to_owned(), upstream.as_str().to_owned()))
        .collect()
}

#[async_trait::async_trait]
impl AuthStore for AuthStoreAdapter {
    async fn load_password_hash(&self, admin_user_id: &str) -> AdminStoreResult<Option<String>> {
        self.identity
            .user_by_id(admin_user_id)
            .await
            .map(|user| user.map(|user| user.password_hash))
            .map_err(|error| admin_store_error("admin authentication", error))
    }

    async fn change_password(
        &self,
        admin_user_id: &str,
        expected_hash: &str,
        password_hash: &str,
        audit: AdminAuditModel,
    ) -> AdminStoreResult<bool> {
        self.identity
            .change_password(
                admin_user_id,
                expected_hash,
                password_hash,
                auth_audit_record(audit)?,
            )
            .await
            .map_err(|error| admin_store_error("administrator password", error))
    }

    async fn create_password_hash_if_absent(
        &self,
        admin_user_id: &str,
        password_hash: &str,
    ) -> AdminStoreResult<bool> {
        self.identity
            .ensure_default_admin(admin_user_id, password_hash)
            .await
            .map_err(|error| admin_store_error("admin authentication", error))
    }

    async fn load_user_by_username(
        &self,
        username: &str,
    ) -> AdminStoreResult<Option<gateway_admin::model::users::UserCredentialRecord>> {
        self.identity
            .user_by_username(username)
            .await
            .map_err(|error| admin_store_error("user identity", error))?
            .map(identity_user_credential)
            .transpose()
    }

    async fn load_user_by_id(
        &self,
        user_id: &str,
    ) -> AdminStoreResult<Option<gateway_admin::model::users::UserCredentialRecord>> {
        self.identity
            .user_by_id(user_id)
            .await
            .map_err(|error| admin_store_error("user identity", error))?
            .map(identity_user_credential)
            .transpose()
    }

    async fn load_user_key_identity(
        &self,
        user_id: &str,
    ) -> AdminStoreResult<gateway_admin::model::users::UserKeyIdentity> {
        self.identity
            .key_identity(user_id)
            .await
            .map_err(|error| admin_store_error("user key identity", error))
    }

    async fn replace_user_key_identity(
        &self,
        user_id: &str,
        identity: gateway_admin::model::users::UserKeyIdentity,
        context: &MutationContext,
    ) -> AdminStoreResult<gateway_admin::model::Revision> {
        let revision = self
            .identity
            .replace_key_identity(
                user_id,
                identity,
                mutation_audit(
                    context,
                    "user.key_identity.update",
                    "user",
                    user_id,
                    vec!["provider_request_profiles".to_owned()],
                ),
            )
            .await
            .map_err(|error| admin_store_error("user key identity", error))?;
        admin_revision(revision)
    }

    async fn list_users(&self) -> AdminStoreResult<Vec<gateway_admin::model::users::UserRecord>> {
        self.identity
            .list_users()
            .await
            .map_err(|error| admin_store_error("user identity", error))?
            .into_iter()
            .map(identity_user_record)
            .collect()
    }

    async fn create_user(
        &self,
        user: gateway_admin::model::users::UserCredentialRecord,
        context: &MutationContext,
    ) -> AdminStoreResult<gateway_admin::model::users::UserRecord> {
        let stored = stored_user(&user);
        self.identity
            .create_user(
                &stored,
                mutation_audit(
                    context,
                    "user.create",
                    "user",
                    &stored.id,
                    vec![
                        "username".to_owned(),
                        "role".to_owned(),
                        "enabled".to_owned(),
                    ],
                ),
            )
            .await
            .map_err(|error| admin_store_error("user identity", error))?;
        Ok(user.user)
    }

    async fn update_user(
        &self,
        command: &gateway_admin::model::users::UpdateUser,
        context: &MutationContext,
    ) -> AdminStoreResult<(
        gateway_admin::model::Revision,
        gateway_admin::model::users::UserRecord,
    )> {
        let mut changed_fields = Vec::new();
        if command.username.is_some() {
            changed_fields.push("username".to_owned());
        }
        if command.role.is_some() {
            changed_fields.push("role".to_owned());
        }
        if command.enabled.is_some() {
            changed_fields.push("enabled".to_owned());
        }
        if command.max_concurrency.is_some() {
            changed_fields.push("max_concurrency".to_owned());
        }
        if command.requests_per_minute.is_some() {
            changed_fields.push("requests_per_minute".to_owned());
        }
        let (revision, stored) = self
            .identity
            .update_user(
                postgres::StoredUserUpdate {
                    id: &command.user_id,
                    username: command.username.as_deref(),
                    role: command
                        .role
                        .map(gateway_admin::model::users::UserRole::as_str),
                    enabled: command.enabled,
                    max_concurrency: command.max_concurrency,
                    requests_per_minute: command.requests_per_minute,
                },
                mutation_audit(
                    context,
                    "user.update",
                    "user",
                    &command.user_id,
                    changed_fields,
                ),
            )
            .await
            .map_err(|error| admin_store_error("user identity", error))?;
        Ok((admin_revision(revision)?, identity_user_record(stored)?))
    }

    async fn update_user_username(
        &self,
        user_id: &str,
        username: &str,
        expected_session_version: u64,
    ) -> AdminStoreResult<Option<gateway_admin::model::users::UserRecord>> {
        self.identity
            .update_username(user_id, username, expected_session_version)
            .await
            .map_err(|error| admin_store_error("user identity", error))?
            .map(identity_user_record)
            .transpose()
    }

    async fn update_user_password_hash_if_matches(
        &self,
        user_id: &str,
        expected_hash: &str,
        password_hash: &str,
    ) -> AdminStoreResult<bool> {
        self.identity
            .update_password_hash_if_matches(user_id, expected_hash, password_hash)
            .await
            .map_err(|error| admin_store_error("user password", error))
    }

    async fn reset_user_password_hash(
        &self,
        user_id: &str,
        password_hash: &str,
        context: &MutationContext,
    ) -> AdminStoreResult<gateway_admin::model::users::UserRecord> {
        let stored = self
            .identity
            .reset_password_hash(
                user_id,
                password_hash,
                mutation_audit(
                    context,
                    "user.password.reset",
                    "user",
                    user_id,
                    vec!["password".to_owned(), "session_version".to_owned()],
                ),
            )
            .await
            .map_err(|error| admin_store_error("user password", error))?;
        identity_user_record(stored)
    }

    async fn bump_user_session_version(
        &self,
        user_id: &str,
        context: &MutationContext,
    ) -> AdminStoreResult<gateway_admin::model::users::UserRecord> {
        let stored = self
            .identity
            .bump_session_version(
                user_id,
                mutation_audit(
                    context,
                    "user.sessions.revoke",
                    "user",
                    user_id,
                    vec!["session_version".to_owned()],
                ),
            )
            .await
            .map_err(|error| admin_store_error("user sessions", error))?;
        identity_user_record(stored)
    }

    async fn delete_user(
        &self,
        user_id: &str,
        context: &MutationContext,
    ) -> AdminStoreResult<gateway_admin::model::Revision> {
        self.identity
            .delete_user(
                user_id,
                mutation_audit(
                    context,
                    "user.delete",
                    "user",
                    user_id,
                    vec![
                        "user".to_owned(),
                        "owned_client_keys".to_owned(),
                        "groups".to_owned(),
                        "budget_windows".to_owned(),
                    ],
                ),
            )
            .await
            .map(admin_revision)
            .map_err(|error| admin_store_error("user identity", error))?
    }

    async fn load_session_ttl_settings(
        &self,
    ) -> AdminStoreResult<Option<gateway_admin::model::users::SessionTtlSettings>> {
        self.identity
            .session_ttl_settings()
            .await
            .map(|settings| {
                settings.map(|(admin_minutes, user_minutes)| {
                    gateway_admin::model::users::SessionTtlSettings::new(
                        admin_minutes,
                        user_minutes,
                    )
                })
            })
            .map_err(|error| admin_store_error("auth settings", error))
    }

    async fn replace_session_ttl_settings(
        &self,
        settings: gateway_admin::model::users::SessionTtlSettings,
        context: &MutationContext,
    ) -> AdminStoreResult<gateway_admin::model::users::SessionTtlSettings> {
        self.identity
            .replace_session_ttl_settings(
                settings.admin_minutes,
                settings.user_minutes,
                mutation_audit(
                    context,
                    "auth.session_ttl.update",
                    "auth_settings",
                    "1",
                    vec![
                        "admin_session_ttl_minutes".to_owned(),
                        "user_session_ttl_minutes".to_owned(),
                    ],
                ),
            )
            .await
            .map_err(|error| admin_store_error("auth settings", error))?;
        Ok(settings)
    }

    async fn load_turnstile_settings(
        &self,
    ) -> AdminStoreResult<gateway_admin::model::users::TurnstileSettings> {
        self.identity
            .auth_settings()
            .await
            .map(|settings| {
                gateway_admin::model::users::TurnstileSettings::new(
                    settings.turnstile_enabled,
                    settings.turnstile_site_key,
                    settings.turnstile_secret_key,
                )
            })
            .map_err(|error| admin_store_error("auth settings", error))
    }

    async fn replace_turnstile_settings(
        &self,
        settings: gateway_admin::model::users::TurnstileSettings,
        context: &MutationContext,
    ) -> AdminStoreResult<gateway_admin::model::users::TurnstileSettings> {
        let stored = postgres::StoredAuthSettings {
            turnstile_enabled: settings.enabled,
            turnstile_site_key: settings.site_key.clone(),
            turnstile_secret_key: settings.secret_for_store().map(str::to_owned),
        };
        self.identity
            .replace_auth_settings(
                &stored,
                mutation_audit(
                    context,
                    "auth.turnstile.update",
                    "auth_settings",
                    "1",
                    vec![
                        "turnstile_enabled".to_owned(),
                        "turnstile_site_key".to_owned(),
                        "turnstile_secret_key".to_owned(),
                    ],
                ),
            )
            .await
            .map_err(|error| admin_store_error("auth settings", error))?;
        Ok(settings)
    }

    async fn load_admin_api_key(&self) -> AdminStoreResult<Option<AdminApiKey>> {
        postgres::RuntimeSettingsRepository::load_runtime_settings(&self.settings)
            .await
            .map(|settings| settings.admin_api_key.map(AdminApiKey::new))
            .map_err(|error| admin_store_error("admin API key", error))
    }

    async fn load_session(&self, session_id: &str) -> AdminStoreResult<Option<AuthSession>> {
        redis::AuthStateRepository::load_session(&self.state, session_id)
            .await
            .map_err(|error| admin_store_error("authentication session", error))?
            .map(auth_session)
            .transpose()
    }

    async fn store_session(&self, session_id: &str, session: &AuthSession) -> AdminStoreResult<()> {
        redis::AuthStateRepository::store_session(
            &self.state,
            session_id,
            &auth_session_record(session),
        )
        .await
        .map_err(|error| admin_store_error("authentication session", error))
    }

    async fn replace_session_if_matches(
        &self,
        session_id: &str,
        expected: &AuthSession,
        replacement: &AuthSession,
    ) -> AdminStoreResult<bool> {
        redis::AuthStateRepository::replace_session_if_matches(
            &self.state,
            session_id,
            &auth_session_record(expected),
            &auth_session_record(replacement),
        )
        .await
        .map_err(|error| admin_store_error("authentication session", error))
    }

    async fn delete_session(&self, session_id: &str) -> AdminStoreResult<Option<AuthSession>> {
        redis::AuthStateRepository::delete_session(&self.state, session_id)
            .await
            .map_err(|error| admin_store_error("authentication session", error))?
            .map(auth_session)
            .transpose()
    }

    async fn client_key_enabled(
        &self,
        id: &gateway_core::policy::ClientApiKeyId,
    ) -> AdminStoreResult<bool> {
        self.keys.is_enabled(id).await
    }

    async fn consume_login_attempt(
        &self,
        source_ip: std::net::IpAddr,
        source_limit: u32,
        global_limit: u32,
        window: std::time::Duration,
    ) -> AdminStoreResult<Option<std::time::Duration>> {
        redis::AuthStateRepository::consume_login_attempt(
            &self.state,
            &source_ip.to_string(),
            source_limit,
            global_limit,
            window,
        )
        .await
        .map_err(|error| admin_store_error("login limit", error))
    }

    async fn append_audit_event(&self, event: AdminAuditModel) -> AdminStoreResult<()> {
        postgres::AdminSecurityAuditRepository::append_admin_audit_event(
            &self.security,
            auth_audit_record(event)?,
        )
        .await
        .map_err(|error| admin_store_error("admin audit", error))
    }
}

fn identity_user_record(
    stored: postgres::StoredUser,
) -> AdminStoreResult<gateway_admin::model::users::UserRecord> {
    let role = gateway_admin::model::users::UserRole::parse(&stored.role).ok_or_else(|| {
        AdminStoreError::new(
            AdminStoreErrorKind::Invalid,
            "user identity",
            "stored user role is invalid",
        )
    })?;
    Ok(gateway_admin::model::users::UserRecord {
        id: stored.id,
        username: stored.username,
        role,
        enabled: stored.enabled,
        limits: UserRateLimits {
            max_concurrency: stored
                .max_concurrency
                .map(u64::try_from)
                .transpose()
                .map_err(|_| {
                    AdminStoreError::new(
                        AdminStoreErrorKind::Invalid,
                        "user identity",
                        "stored user max concurrency is invalid",
                    )
                })?,
            requests_per_minute: stored
                .requests_per_minute
                .map(u64::try_from)
                .transpose()
                .map_err(|_| {
                    AdminStoreError::new(
                        AdminStoreErrorKind::Invalid,
                        "user identity",
                        "stored user requests per minute is invalid",
                    )
                })?,
        },
        session_version: u64::try_from(stored.session_version).map_err(|_| {
            AdminStoreError::new(
                AdminStoreErrorKind::Invalid,
                "user identity",
                "stored session version is invalid",
            )
        })?,
        created_at: stored.created_at,
        updated_at: stored.updated_at,
    })
}

fn identity_user_credential(
    stored: postgres::StoredUser,
) -> AdminStoreResult<gateway_admin::model::users::UserCredentialRecord> {
    let password_hash = stored.password_hash.clone();
    Ok(gateway_admin::model::users::UserCredentialRecord {
        user: identity_user_record(stored)?,
        password_hash,
    })
}

fn stored_user(user: &gateway_admin::model::users::UserCredentialRecord) -> postgres::StoredUser {
    postgres::StoredUser {
        id: user.user.id.clone(),
        username: user.user.username.clone(),
        password_hash: user.password_hash.clone(),
        role: user.user.role.as_str().to_owned(),
        enabled: user.user.enabled,
        max_concurrency: user
            .user
            .limits
            .max_concurrency
            .map(|value| i64::try_from(value).unwrap_or(i64::MAX)),
        requests_per_minute: user
            .user
            .limits
            .requests_per_minute
            .map(|value| i64::try_from(value).unwrap_or(i64::MAX)),
        session_version: i64::try_from(user.user.session_version).unwrap_or(i64::MAX),
        created_at: user.user.created_at,
        updated_at: user.user.updated_at,
    }
}

fn auth_audit_record(event: AdminAuditModel) -> AdminStoreResult<postgres::AdminAuditEvent> {
    let config_revision = event
        .config_revision
        .map(|revision| i64::try_from(revision.get()))
        .transpose()
        .map_err(|_| {
            AdminStoreError::new(
                AdminStoreErrorKind::Invalid,
                "admin audit",
                "config revision is outside the supported range",
            )
        })?;
    let actor_kind = match event.actor_kind {
        gateway_admin::model::auth::AuditActorKind::UserSession => {
            postgres::AdminAuditActorKind::UserSession
        }
        gateway_admin::model::auth::AuditActorKind::AdminSession => {
            postgres::AdminAuditActorKind::AdminSession
        }
        gateway_admin::model::auth::AuditActorKind::AdminApiKey => {
            postgres::AdminAuditActorKind::AdminApiKey
        }
        gateway_admin::model::auth::AuditActorKind::System => postgres::AdminAuditActorKind::System,
        gateway_admin::model::auth::AuditActorKind::Anonymous => {
            postgres::AdminAuditActorKind::Anonymous
        }
    };
    Ok(postgres::AdminAuditEvent {
        id: event.id,
        actor_kind,
        actor_admin_user_id: event.actor_admin_user_id,
        actor_ref: event.actor_ref,
        admin_request_id: event.request_id,
        action: event.action,
        entity_kind: event.entity_kind,
        entity_ref: event.entity_ref,
        config_revision,
        changed_fields: event.changed_fields,
        created_at: event.occurred_at,
    })
}

fn auth_session_record(session: &AuthSession) -> redis::AuthSessionRecord {
    let subject = match &session.subject {
        SessionSubject::Admin {
            admin_user_id,
            credential_fingerprint,
        } => redis::SessionSubjectRecord::Admin {
            admin_user_id: admin_user_id.clone(),
            credential_fingerprint: credential_fingerprint.clone(),
        },
        SessionSubject::User {
            user_id,
            credential_fingerprint,
        } => redis::SessionSubjectRecord::User {
            user_id: user_id.clone(),
            credential_fingerprint: credential_fingerprint.clone(),
        },
        SessionSubject::Key { client_key_id } => redis::SessionSubjectRecord::Key {
            client_key_id: client_key_id.as_str().to_owned(),
        },
    };
    redis::AuthSessionRecord {
        subject,
        expires_at: session.expires_at,
    }
}

fn auth_session(record: redis::AuthSessionRecord) -> AdminStoreResult<AuthSession> {
    let subject = match record.subject {
        redis::SessionSubjectRecord::Admin {
            admin_user_id,
            credential_fingerprint,
        } => SessionSubject::Admin {
            admin_user_id,
            credential_fingerprint,
        },
        redis::SessionSubjectRecord::User {
            user_id,
            credential_fingerprint,
        } => SessionSubject::User {
            user_id,
            credential_fingerprint,
        },
        redis::SessionSubjectRecord::Key { client_key_id } => SessionSubject::Key {
            client_key_id: gateway_core::policy::ClientApiKeyId::new(client_key_id).map_err(
                |_| {
                    AdminStoreError::new(
                        AdminStoreErrorKind::Invalid,
                        "authentication session",
                        "client key ID is invalid",
                    )
                },
            )?,
        },
    };
    Ok(AuthSession {
        subject,
        expires_at: record.expires_at,
    })
}
