//! `PgPropertyDefinitionRepo` only. `PgProjectRepo`/`PgFolderRepo` stay in
//! `atlas_server::persistence::repos::workspace_core` (S4 PR8): their
//! `soft_delete` (and, for `PgFolderRepo`, `create`) methods compose a Custos
//! audit-log write via `append_resource_deleted_in`/`PgOutboxRepo`, the same
//! boundary PR7 already documented for `PgAttachmentRepo`/
//! `PgAttachmentLifecycle` — the whole trait impl for a straddling struct
//! stays server-side rather than being split method-by-method, since a
//! trait's `impl` block cannot itself be split across crates.
//!
//! `user_id_from_actor` and the local `db_err` wrapper (unique-constraint ->
//! `AlreadyExists` mapping) are duplicated here rather than shared: both are
//! small, pure, dependency-free helpers also needed by the staying
//! `PgProjectRepo`/`PgFolderRepo`, and their two call-site groups now live in
//! different crates (same pattern as PR7's `actor_fields` duplication).

use async_trait::async_trait;
use atlas_acta::actor::Actor;
use atlas_acta::actor::WorkspaceCtx;
use atlas_acta::entities::workspace_core::NewPropertyDefinition;
use atlas_acta::entities::workspace_core::PropertyDefinition;
use atlas_acta::ids::PropertyDefinitionId;
use atlas_core::error::DomainError;
use atlas_postgres::db_err as internal_db_err;
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait,
    IntoActiveModel, QueryFilter,
};

use crate::entities::workspace_core::{property_definition, property_definition_from};

pub use atlas_acta::ports::workspace_core::PropertyDefinitionRepo;

pub struct PgPropertyDefinitionRepo {
    pub conn: DatabaseConnection,
}

#[async_trait]
impl PropertyDefinitionRepo for PgPropertyDefinitionRepo {
    async fn create(
        &self,
        ctx: &WorkspaceCtx,
        new: NewPropertyDefinition,
    ) -> Result<PropertyDefinition, DomainError> {
        let created_by_user_id = user_id_from_actor(&ctx.actor);
        let model = property_definition::ActiveModel {
            id: Set(PropertyDefinitionId::new().0),
            workspace_id: Set(ctx.workspace_id.0),
            key: Set(new.key),
            name: Set(new.name),
            kind: Set(new.kind.as_str().to_string()),
            options: Set(new.options),
            applies_to: Set(new.applies_to.as_str().to_string()),
            created_by_user_id: Set(created_by_user_id),
            created_by_api_key_id: Set(None),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            deleted_at: Set(None),
        };
        model
            .insert(&self.conn)
            .await
            .map_err(db_err)
            .and_then(|m| property_definition_from(m).map_err(internal_err))
    }

    async fn find(
        &self,
        ctx: &WorkspaceCtx,
        id: PropertyDefinitionId,
    ) -> Result<Option<PropertyDefinition>, DomainError> {
        property_definition::Entity::find_by_id(id.0)
            .filter(property_definition::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(property_definition::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .map(property_definition_from)
            .transpose()
            .map_err(internal_err)
    }

    async fn list(&self, ctx: &WorkspaceCtx) -> Result<Vec<PropertyDefinition>, DomainError> {
        let rows = property_definition::Entity::find()
            .filter(property_definition::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(property_definition::Column::DeletedAt.is_null())
            .all(&self.conn)
            .await
            .map_err(db_err)?;

        rows.into_iter()
            .map(|m| property_definition_from(m).map_err(internal_err))
            .collect()
    }

    async fn soft_delete(
        &self,
        ctx: &WorkspaceCtx,
        id: PropertyDefinitionId,
    ) -> Result<(), DomainError> {
        let row = property_definition::Entity::find_by_id(id.0)
            .filter(property_definition::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(property_definition::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "property_definition",
                id: id.0,
            })?;

        let mut active = row.into_active_model();
        active.deleted_at = Set(Some(Utc::now()));
        active.update(&self.conn).await.map_err(db_err)?;
        Ok(())
    }
}

fn user_id_from_actor(actor: &Actor) -> Option<uuid::Uuid> {
    match actor {
        Actor::User(uid) => Some(uid.0),
        Actor::ApiKey(_) => None,
    }
}

fn db_err(e: sea_orm::DbErr) -> DomainError {
    // A unique-constraint violation is a caller error (e.g. a property
    // definition with that key already exists in the workspace), not an
    // internal fault: surface it as a 409 instead of an opaque 500.
    if let Some(sea_orm::SqlErr::UniqueConstraintViolation(_)) = e.sql_err() {
        return DomainError::AlreadyExists {
            message: "an item with the same name already exists in this location".to_string(),
        };
    }

    internal_db_err(e)
}

fn internal_err(msg: String) -> DomainError {
    DomainError::Internal { message: msg }
}
