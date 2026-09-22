//! Client API Key 管理用例。

use std::sync::Arc;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use gateway_core::policy::ClientApiKeyId;
use gateway_core::runtime::SnapshotControl;
use rand_core::{OsRng, RngCore as _};
use uuid::Uuid;

use crate::{
    model::{
        AdminError,
        client_keys::{
            ClientKeyCursorValue, ClientKeyListQuery, ClientKeyMutation, ClientKeyPage,
            ClientKeySecret, ClientKeySortField, CreateClientKey, CreatedClientKey,
            DeleteClientKey, NewClientKey, ReplaceClientKeyIdentity, ResetClientKeyBudget,
            SetClientKeyEnabled, UpdateClientKey,
        },
    },
    ports::store::{AdminStoreError, AdminStoreErrorKind, ClientKeyStore},
};

use super::{map_store_error, publish_committed};

/// API 消费的 Client Key 管理服务。
#[async_trait]
pub trait ClientKeyService: Send + Sync {
    async fn reset_budget_for_user(
        &self,
        user_id: &str,
        command: ResetClientKeyBudget,
    ) -> Result<ClientApiKeyId, AdminError>;
    async fn list_for_user(
        &self,
        user_id: &str,
        query: ClientKeyListQuery,
    ) -> Result<ClientKeyPage, AdminError>;
    async fn reveal_for_user(
        &self,
        user_id: &str,
        id: &ClientApiKeyId,
    ) -> Result<ClientKeySecret, AdminError>;
    async fn replace_identity_for_user(
        &self,
        context: &crate::model::MutationContext,
        user_id: &str,
        command: ReplaceClientKeyIdentity,
    ) -> Result<ClientKeyMutation, AdminError>;
    async fn create_for_user(
        &self,
        user_id: &str,
        command: CreateClientKey,
    ) -> Result<CreatedClientKey, AdminError>;
    async fn update_for_user(
        &self,
        user_id: &str,
        command: UpdateClientKey,
    ) -> Result<ClientKeyMutation, AdminError>;
    async fn set_enabled_for_user(
        &self,
        user_id: &str,
        command: SetClientKeyEnabled,
    ) -> Result<ClientKeyMutation, AdminError>;
    async fn delete_for_user(
        &self,
        user_id: &str,
        command: DeleteClientKey,
    ) -> Result<ClientKeyMutation, AdminError>;
}

pub(crate) struct DefaultClientKeyService {
    store: Arc<dyn ClientKeyStore>,
    snapshot: Arc<dyn SnapshotControl>,
}

impl DefaultClientKeyService {
    #[must_use]
    pub(crate) fn new(store: Arc<dyn ClientKeyStore>, snapshot: Arc<dyn SnapshotControl>) -> Self {
        Self { store, snapshot }
    }
}

#[async_trait]
impl ClientKeyService for DefaultClientKeyService {
    async fn reset_budget_for_user(
        &self,
        user_id: &str,
        command: ResetClientKeyBudget,
    ) -> Result<ClientApiKeyId, AdminError> {
        let id = command.id.clone();
        self.store
            .reset_user_client_key_budget(user_id, command)
            .await
            .map_err(|error| map_store_error(error, "client API key"))?;
        Ok(id)
    }

    async fn list_for_user(
        &self,
        user_id: &str,
        query: ClientKeyListQuery,
    ) -> Result<ClientKeyPage, AdminError> {
        validate_cursor(&query)?;
        self.store
            .list_user_client_keys(user_id, query)
            .await
            .map_err(|error| map_store_error(error, "client API key"))
    }

    async fn reveal_for_user(
        &self,
        user_id: &str,
        id: &ClientApiKeyId,
    ) -> Result<ClientKeySecret, AdminError> {
        self.store
            .reveal_user_client_key(user_id, id)
            .await
            .map_err(|error| map_store_error(error, "client API key"))?
            .ok_or_else(|| AdminError::not_found("Client API Key 不存在"))
    }

    async fn replace_identity_for_user(
        &self,
        context: &crate::model::MutationContext,
        user_id: &str,
        command: ReplaceClientKeyIdentity,
    ) -> Result<ClientKeyMutation, AdminError> {
        let id = command.id.clone();
        let (config_revision, record) = self
            .store
            .replace_user_client_key_identity(user_id, command, context)
            .await
            .map_err(|error| map_store_error(error, "client API key"))?;
        publish_committed(self.snapshot.as_ref(), config_revision).await?;
        Ok(ClientKeyMutation {
            config_revision,
            record: Some(record),
            id,
        })
    }

    async fn create_for_user(
        &self,
        user_id: &str,
        command: CreateClientKey,
    ) -> Result<CreatedClientKey, AdminError> {
        if !command.group_ids.is_empty()
            || command.openai_client_profile_override.is_some()
            || command.xai_client_profile_override.is_some()
        {
            return Err(AdminError::invalid("普通用户 API Key 不接受账号分组配置"));
        }
        let id = ClientApiKeyId::new(format!("key_{}", Uuid::now_v7().simple()))
            .map_err(|_| AdminError::internal("创建 Client API Key ID 失败"))?;
        let plaintext = if let Some(key) = command.custom_key {
            key.expose_for_auth().to_owned()
        } else {
            let mut bytes = [0_u8; 32];
            OsRng.fill_bytes(&mut bytes);
            format!("sk_{}", URL_SAFE_NO_PAD.encode(bytes))
        };
        let (config_revision, record) = self
            .store
            .create_user_client_key(
                user_id,
                NewClientKey {
                    openai_client_profile_override: command.openai_client_profile_override,
                    xai_client_profile_override: command.xai_client_profile_override,
                    id,
                    name: command.name,
                    label: command.label,
                    group_ids: command.group_ids,
                    limits: command.limits,
                    budget: command.budget,
                    plaintext: plaintext.clone(),
                },
            )
            .await
            .map_err(map_client_key_write_error)?;
        publish_committed(self.snapshot.as_ref(), config_revision).await?;
        Ok(CreatedClientKey {
            config_revision,
            secret: ClientKeySecret::new(record, plaintext),
        })
    }

    async fn update_for_user(
        &self,
        user_id: &str,
        command: UpdateClientKey,
    ) -> Result<ClientKeyMutation, AdminError> {
        if !command.group_ids.is_empty()
            || command.openai_client_profile_override.is_some()
            || command.xai_client_profile_override.is_some()
        {
            return Err(AdminError::invalid("普通用户 API Key 不接受账号分组配置"));
        }
        let id = command.id.clone();
        let (config_revision, record) = self
            .store
            .update_user_client_key(user_id, command)
            .await
            .map_err(map_client_key_write_error)?;
        publish_committed(self.snapshot.as_ref(), config_revision).await?;
        Ok(ClientKeyMutation {
            config_revision,
            record: Some(record),
            id,
        })
    }
    async fn set_enabled_for_user(
        &self,
        user_id: &str,
        command: SetClientKeyEnabled,
    ) -> Result<ClientKeyMutation, AdminError> {
        let id = command.id.clone();
        let (config_revision, record) = self
            .store
            .set_user_client_key_enabled(user_id, command)
            .await
            .map_err(|error| map_store_error(error, "client API key"))?;
        publish_committed(self.snapshot.as_ref(), config_revision).await?;
        Ok(ClientKeyMutation {
            config_revision,
            record: Some(record),
            id,
        })
    }
    async fn delete_for_user(
        &self,
        user_id: &str,
        command: DeleteClientKey,
    ) -> Result<ClientKeyMutation, AdminError> {
        let id = command.id.clone();
        let config_revision = self
            .store
            .delete_user_client_key(user_id, command)
            .await
            .map_err(|error| map_store_error(error, "client API key"))?;
        publish_committed(self.snapshot.as_ref(), config_revision).await?;
        Ok(ClientKeyMutation {
            config_revision,
            record: None,
            id,
        })
    }
}

fn map_client_key_write_error(error: AdminStoreError) -> AdminError {
    match error.kind() {
        AdminStoreErrorKind::DuplicateName => AdminError::conflict("名称已存在"),
        AdminStoreErrorKind::Conflict => AdminError::conflict("API Key 已存在，请使用其他密钥"),
        _ => map_store_error(error, "client API key"),
    }
}

fn validate_cursor(query: &ClientKeyListQuery) -> Result<(), AdminError> {
    let Some(cursor) = &query.cursor else {
        return Ok(());
    };
    if cursor.sort != query.sort {
        return Err(AdminError::invalid(
            "Client API Key 游标排序与查询条件不一致",
        ));
    }
    let matches = matches!(
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
    if matches {
        Ok(())
    } else {
        Err(AdminError::invalid("Client API Key 游标不合法"))
    }
}
