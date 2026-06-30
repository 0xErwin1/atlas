use atlas_domain::{Actor, DomainError};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, EntityTrait, IntoActiveModel,
    QueryFilter, QueryOrder, QuerySelect,
};
use uuid::Uuid;

use crate::persistence::entities::webhook_subscription::webhook_subscriptions;

pub struct PgWebhookSubscriptionRepo;

/// Fields that may be updated on an existing subscription. `None` leaves the
/// field unchanged; `Some(None)` clears a nullable field.
pub struct WebhookSubscriptionPatch {
    pub target_url: Option<String>,
    pub event_types: Option<Vec<String>>,
    pub scope_type: Option<String>,
    pub scope_id: Option<Option<Uuid>>,
    pub encrypted_secret: Option<Vec<u8>>,
    pub secret_nonce: Option<Vec<u8>>,
    pub is_active: Option<bool>,
    pub label: Option<Option<String>>,
}

impl PgWebhookSubscriptionRepo {
    /// Inserts a new webhook subscription and returns the persisted row.
    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        conn: &impl ConnectionTrait,
        workspace_id: Uuid,
        target_url: String,
        event_types: Vec<String>,
        scope_type: String,
        scope_id: Option<Uuid>,
        encrypted_secret: Vec<u8>,
        secret_nonce: Vec<u8>,
        label: Option<String>,
        actor: &Actor,
    ) -> Result<webhook_subscriptions::Model, DomainError> {
        let (by_user, by_key) = actor_fields(actor);
        let now = Utc::now();

        let model = webhook_subscriptions::ActiveModel {
            id: Set(Uuid::now_v7()),
            workspace_id: Set(workspace_id),
            target_url: Set(target_url),
            event_types: Set(event_types),
            scope_type: Set(scope_type),
            scope_id: Set(scope_id),
            encrypted_secret: Set(encrypted_secret),
            secret_nonce: Set(secret_nonce),
            is_active: Set(true),
            label: Set(label),
            created_by_user_id: Set(by_user),
            created_by_api_key_id: Set(by_key),
            created_at: Set(now),
            updated_at: Set(now),
            deleted_at: Set(None),
        };

        model.insert(conn).await.map_err(db_err)
    }

    /// Looks up a single non-deleted subscription by its UUID within a workspace.
    pub async fn get_by_id(
        conn: &impl ConnectionTrait,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<Option<webhook_subscriptions::Model>, DomainError> {
        webhook_subscriptions::Entity::find_by_id(id)
            .filter(webhook_subscriptions::Column::WorkspaceId.eq(workspace_id))
            .filter(webhook_subscriptions::Column::DeletedAt.is_null())
            .one(conn)
            .await
            .map_err(db_err)
    }

    /// Returns the active (non-deleted) subscriptions in a workspace, paginated
    /// by UUIDv7 cursor.
    pub async fn list_active(
        conn: &impl ConnectionTrait,
        workspace_id: Uuid,
        after_id: Option<Uuid>,
        limit: u64,
    ) -> Result<Vec<webhook_subscriptions::Model>, DomainError> {
        let mut q = webhook_subscriptions::Entity::find()
            .filter(webhook_subscriptions::Column::WorkspaceId.eq(workspace_id))
            .filter(webhook_subscriptions::Column::DeletedAt.is_null());

        if let Some(cursor) = after_id {
            q = q.filter(webhook_subscriptions::Column::Id.gt(cursor));
        }

        q.order_by_asc(webhook_subscriptions::Column::Id)
            .limit(limit)
            .all(conn)
            .await
            .map_err(db_err)
    }

    /// Applies a partial update to an existing subscription. Fields absent from
    /// `patch` are left unchanged.
    pub async fn update(
        conn: &impl ConnectionTrait,
        workspace_id: Uuid,
        id: Uuid,
        patch: WebhookSubscriptionPatch,
    ) -> Result<webhook_subscriptions::Model, DomainError> {
        let row = webhook_subscriptions::Entity::find_by_id(id)
            .filter(webhook_subscriptions::Column::WorkspaceId.eq(workspace_id))
            .filter(webhook_subscriptions::Column::DeletedAt.is_null())
            .one(conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "webhook_subscription",
                id,
            })?;

        let mut active = row.into_active_model();

        if let Some(url) = patch.target_url {
            active.target_url = Set(url);
        }

        if let Some(types) = patch.event_types {
            active.event_types = Set(types);
        }

        if let Some(st) = patch.scope_type {
            active.scope_type = Set(st);
        }

        if let Some(sid) = patch.scope_id {
            active.scope_id = Set(sid);
        }

        if let Some(sec) = patch.encrypted_secret {
            active.encrypted_secret = Set(sec);
        }

        if let Some(nonce) = patch.secret_nonce {
            active.secret_nonce = Set(nonce);
        }

        if let Some(active_flag) = patch.is_active {
            active.is_active = Set(active_flag);
        }

        if let Some(label) = patch.label {
            active.label = Set(label);
        }

        active.updated_at = Set(Utc::now());
        active.update(conn).await.map_err(db_err)
    }

    /// Soft-deletes a subscription by setting `deleted_at`.
    pub async fn soft_delete(
        conn: &impl ConnectionTrait,
        workspace_id: Uuid,
        id: Uuid,
    ) -> Result<(), DomainError> {
        let row = webhook_subscriptions::Entity::find_by_id(id)
            .filter(webhook_subscriptions::Column::WorkspaceId.eq(workspace_id))
            .filter(webhook_subscriptions::Column::DeletedAt.is_null())
            .one(conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "webhook_subscription",
                id,
            })?;

        let mut active = row.into_active_model();
        active.deleted_at = Set(Some(Utc::now()));
        active.updated_at = Set(Utc::now());
        active.update(conn).await.map_err(db_err)?;
        Ok(())
    }
}

fn actor_fields(actor: &Actor) -> (Option<Uuid>, Option<Uuid>) {
    match actor {
        Actor::User(uid) => (Some(uid.0), None),
        Actor::ApiKey(kid) => (None, Some(kid.0)),
    }
}

fn db_err(e: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: e.to_string(),
    }
}
