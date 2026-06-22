use async_trait::async_trait;
use atlas_domain::{
    Actor, DomainError, WorkspaceCtx,
    entities::tags::{NewTag, Tag},
    ids::TagId,
};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ConnectionTrait, DatabaseConnection, EntityTrait,
    FromQueryResult, SqlErr, Statement, TransactionTrait,
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
            color: Set(None),
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
                "SELECT id, workspace_id, name, color, created_by_user_id, \
                 created_by_api_key_id, created_at, updated_at, deleted_at \
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
                "SELECT id, workspace_id, name, color, created_by_user_id, \
                 created_by_api_key_id, created_at, updated_at, deleted_at \
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

    /// Updates the tag's name and/or color in a single transaction.
    ///
    /// On a name change the transaction atomically:
    /// 1. Renames the tag row.
    /// 2. Backfills all non-deleted task labels in the workspace with
    ///    `array_replace(labels, old_name, new_name)`.
    /// 3. Deduplicates labels on any task row where the new name was already
    ///    present before the backfill (which would otherwise produce duplicates).
    ///
    /// A `DomainError::Conflict` is returned when the new name collides with an
    /// existing tag (the unique index `tags_ws_lower_name_idx` raises 23505 which
    /// is mapped here). A `DomainError::NotFound` is returned when the tag does
    /// not exist in this workspace or is already deleted.
    async fn update(
        &self,
        ctx: &WorkspaceCtx,
        id: TagId,
        name: Option<String>,
        color: Option<String>,
    ) -> Result<Tag, DomainError> {
        if name.is_none() && color.is_none() {
            return self.find_by_name_or_err(ctx, id).await;
        }

        let txn = self.conn.begin().await.map_err(db_err)?;

        let existing = load_tag_in_txn(&txn, ctx, id).await?;

        let old_name = existing.name.clone();

        let new_name_ref = name.as_deref().unwrap_or(&old_name);
        let now = Utc::now();

        let update_result = txn
            .execute_raw(Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                "UPDATE tags \
                 SET name = $1, color = COALESCE($2, color), updated_at = $3 \
                 WHERE id = $4 AND workspace_id = $5 AND deleted_at IS NULL",
                [
                    new_name_ref.into(),
                    color.clone().into(),
                    now.into(),
                    id.0.into(),
                    ctx.workspace_id.0.into(),
                ],
            ))
            .await;

        if let Err(e) = update_result {
            let _ = txn.rollback().await;
            match e.sql_err() {
                Some(SqlErr::UniqueConstraintViolation(_)) => {
                    return Err(DomainError::AlreadyExists {
                        message: format!(
                            "a tag with name '{}' already exists in this workspace",
                            new_name_ref
                        ),
                    });
                }
                _ => return Err(db_err(e)),
            }
        }

        if name.is_some() && new_name_ref != old_name {
            txn.execute_raw(Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                "UPDATE tasks \
                 SET labels = array_replace(labels, $1, $2), updated_at = $3 \
                 WHERE workspace_id = $4 AND $1 = ANY(labels) AND deleted_at IS NULL",
                [
                    old_name.clone().into(),
                    new_name_ref.into(),
                    now.into(),
                    ctx.workspace_id.0.into(),
                ],
            ))
            .await
            .map_err(db_err)?;

            txn.execute_raw(Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                "UPDATE tasks \
                 SET labels = ARRAY(SELECT DISTINCT unnest(labels) ORDER BY 1), \
                     updated_at = $1 \
                 WHERE workspace_id = $2 \
                   AND deleted_at IS NULL \
                   AND (SELECT count(*) FROM unnest(labels) l WHERE l = $3) > 1",
                [now.into(), ctx.workspace_id.0.into(), new_name_ref.into()],
            ))
            .await
            .map_err(db_err)?;
        }

        let updated = load_tag_in_txn(&txn, ctx, id).await?;

        txn.commit().await.map_err(db_err)?;

        Ok(updated)
    }

    async fn list_used_labels(&self, ctx: &WorkspaceCtx) -> Result<Vec<String>, DomainError> {
        #[derive(Debug, FromQueryResult)]
        struct LabelRow {
            label: String,
        }

        let rows = LabelRow::find_by_statement(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT DISTINCT unnest(labels) AS label \
             FROM tasks \
             WHERE workspace_id = $1 AND deleted_at IS NULL \
             ORDER BY 1",
            [ctx.workspace_id.0.into()],
        ))
        .all(&self.conn)
        .await
        .map_err(db_err)?;

        Ok(rows.into_iter().map(|r| r.label).collect())
    }

    /// Soft-deletes a tag by setting `deleted_at = now()`.
    ///
    /// Task label strings are not modified — they are free strings without a
    /// foreign key to the tags table.
    async fn soft_delete(&self, ctx: &WorkspaceCtx, id: TagId) -> Result<(), DomainError> {
        let rows_affected = self
            .conn
            .execute_raw(Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                "UPDATE tags \
                 SET deleted_at = $1 \
                 WHERE id = $2 AND workspace_id = $3 AND deleted_at IS NULL",
                [Utc::now().into(), id.0.into(), ctx.workspace_id.0.into()],
            ))
            .await
            .map_err(db_err)?
            .rows_affected();

        if rows_affected == 0 {
            return Err(DomainError::NotFound {
                entity: "tag",
                id: id.0,
            });
        }

        Ok(())
    }
}

async fn load_tag_in_txn<C: ConnectionTrait>(
    conn: &C,
    ctx: &WorkspaceCtx,
    id: TagId,
) -> Result<Tag, DomainError> {
    let model = tag::Entity::find()
        .from_raw_sql(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT id, workspace_id, name, color, created_by_user_id, \
             created_by_api_key_id, created_at, updated_at, deleted_at \
             FROM tags \
             WHERE id = $1 AND workspace_id = $2 AND deleted_at IS NULL \
             LIMIT 1",
            [id.0.into(), ctx.workspace_id.0.into()],
        ))
        .one(conn)
        .await
        .map_err(db_err)?;

    model.map(tag_from).ok_or(DomainError::NotFound {
        entity: "tag",
        id: id.0,
    })
}

impl PgTagRepo {
    async fn find_by_name_or_err(&self, ctx: &WorkspaceCtx, id: TagId) -> Result<Tag, DomainError> {
        load_tag_in_txn(&self.conn, ctx, id).await
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
