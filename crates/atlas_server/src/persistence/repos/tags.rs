use async_trait::async_trait;
use atlas_domain::{
    Actor, DomainError, WorkspaceCtx,
    entities::tags::{NewTag, Tag},
    ids::TagId,
};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, DatabaseConnection, EntityTrait, SqlErr, Statement,
};

use crate::persistence::entities::tags::{tag, tag_from};

pub use atlas_domain::ports::tags::TagRepo;

pub struct PgTagRepo {
    pub conn: DatabaseConnection,
}

impl PgTagRepo {
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl TagRepo for PgTagRepo {
    /// Inserts a new tag. The unique `(workspace_id, lower(name))` index makes a
    /// concurrent duplicate insert fail with 23505; that case is treated as
    /// "already exists" and resolved by returning the existing row, keeping the
    /// create operation idempotent even under a race with the route pre-check.
    async fn create(&self, ctx: &WorkspaceCtx, new: NewTag) -> Result<Tag, DomainError> {
        let (by_user, by_key) = actor_columns(&ctx.actor);

        let model = tag::ActiveModel {
            id: Set(TagId::new().0),
            workspace_id: Set(ctx.workspace_id.0),
            name: Set(new.name.clone()),
            created_by_user_id: Set(by_user),
            created_by_api_key_id: Set(by_key),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            deleted_at: Set(None),
        };

        match model.insert(&self.conn).await {
            Ok(inserted) => Ok(tag_from(inserted)),
            Err(e) => match e.sql_err() {
                Some(SqlErr::UniqueConstraintViolation(_)) => {
                    match self.find_by_name(ctx, &new.name).await? {
                        Some(existing) => Ok(existing),
                        None => Err(db_err(e)),
                    }
                }
                _ => Err(db_err(e)),
            },
        }
    }

    async fn find_by_name(
        &self,
        ctx: &WorkspaceCtx,
        name: &str,
    ) -> Result<Option<Tag>, DomainError> {
        let lower = name.to_lowercase();

        let model = tag::Entity::find()
            .from_raw_sql(Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                "SELECT id, workspace_id, name, created_by_user_id, created_by_api_key_id, \
                 created_at, updated_at, deleted_at \
                 FROM tags \
                 WHERE workspace_id = $1 AND deleted_at IS NULL AND lower(name) = $2 \
                 LIMIT 1",
                [ctx.workspace_id.0.into(), lower.into()],
            ))
            .one(&self.conn)
            .await
            .map_err(db_err)?;

        Ok(model.map(tag_from))
    }

    /// Lists the workspace's non-deleted tags ordered by `lower(name)` so the
    /// ordering is case-insensitive and independent of the database collation.
    async fn list(&self, ctx: &WorkspaceCtx) -> Result<Vec<Tag>, DomainError> {
        tag::Entity::find()
            .from_raw_sql(Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                "SELECT id, workspace_id, name, created_by_user_id, created_by_api_key_id, \
                 created_at, updated_at, deleted_at \
                 FROM tags \
                 WHERE workspace_id = $1 AND deleted_at IS NULL \
                 ORDER BY lower(name) ASC",
                [ctx.workspace_id.0.into()],
            ))
            .all(&self.conn)
            .await
            .map(|rows| rows.into_iter().map(tag_from).collect())
            .map_err(db_err)
    }
}

/// Returns `(created_by_user_id, created_by_api_key_id)` for the XOR actor columns.
fn actor_columns(actor: &Actor) -> (Option<uuid::Uuid>, Option<uuid::Uuid>) {
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
