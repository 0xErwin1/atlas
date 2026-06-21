use async_trait::async_trait;
use atlas_domain::{
    DomainError, TaskViewId, WorkspaceCtx,
    entities::task_views::{NewTaskView, TaskView, TaskViewFilters},
};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, IntoActiveModel, QueryFilter, SqlErr, Statement,
};

use crate::persistence::entities::task_views::{task_view, task_view_from};

pub use atlas_domain::ports::task_views::TaskViewRepo;

pub struct PgTaskViewRepo {
    pub conn: DatabaseConnection,
}

impl PgTaskViewRepo {
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl TaskViewRepo for PgTaskViewRepo {
    /// Creates a new task view for the calling principal.
    ///
    /// The COUNT-before-INSERT cap check at >= 50 is not transactional, so two
    /// concurrent creates at n=49 can both pass and briefly exceed the cap by one.
    /// This is accepted: the cap is an abuse ceiling, not a hard invariant, and
    /// an off-by-one under concurrency is harmless.
    async fn create(&self, ctx: &WorkspaceCtx, new: NewTaskView) -> Result<TaskView, DomainError> {
        let (owner_user_col, owner_key_col) = owner_columns(&ctx.actor);

        let count = self.count_live(ctx).await?;
        if count >= 50 {
            return Err(DomainError::InvalidInput {
                message: "you have reached the maximum of 50 task views for this workspace; \
                          delete one before creating another"
                    .to_string(),
            });
        }

        let filters_value =
            serde_json::to_value(&new.filters).map_err(|e| DomainError::Internal {
                message: format!("failed to serialize task_view filters: {e}"),
            })?;

        let model = task_view::ActiveModel {
            id: Set(TaskViewId::new().0),
            workspace_id: Set(ctx.workspace_id.0),
            name: Set(new.name),
            filters: Set(filters_value),
            owner_user_id: Set(owner_user_col),
            owner_api_key_id: Set(owner_key_col),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            deleted_at: Set(None),
        };

        match model.insert(&self.conn).await {
            Ok(inserted) => task_view_from(inserted),
            Err(e) => match e.sql_err() {
                Some(SqlErr::UniqueConstraintViolation(_)) => Err(DomainError::AlreadyExists {
                    message: "a task view with this name already exists".to_string(),
                }),
                _ => Err(db_err(e)),
            },
        }
    }

    async fn find(
        &self,
        ctx: &WorkspaceCtx,
        id: TaskViewId,
    ) -> Result<Option<TaskView>, DomainError> {
        let model = self.find_raw(ctx, id).await?;
        model.map(task_view_from).transpose()
    }

    async fn list_for_owner(&self, ctx: &WorkspaceCtx) -> Result<Vec<TaskView>, DomainError> {
        let (owner_user_col, owner_key_col) = owner_columns(&ctx.actor);

        let rows = match (owner_user_col, owner_key_col) {
            (Some(uid), None) => task_view::Entity::find()
                .from_raw_sql(Statement::from_sql_and_values(
                    sea_orm::DatabaseBackend::Postgres,
                    "SELECT id, workspace_id, name, filters, owner_user_id, owner_api_key_id, \
                         created_at, updated_at, deleted_at \
                         FROM task_views \
                         WHERE workspace_id = $1 AND owner_user_id = $2 AND deleted_at IS NULL \
                         ORDER BY lower(name) ASC",
                    [ctx.workspace_id.0.into(), uid.into()],
                ))
                .all(&self.conn)
                .await
                .map_err(db_err)?,
            (None, Some(kid)) => task_view::Entity::find()
                .from_raw_sql(Statement::from_sql_and_values(
                    sea_orm::DatabaseBackend::Postgres,
                    "SELECT id, workspace_id, name, filters, owner_user_id, owner_api_key_id, \
                         created_at, updated_at, deleted_at \
                         FROM task_views \
                         WHERE workspace_id = $1 AND owner_api_key_id = $2 AND deleted_at IS NULL \
                         ORDER BY lower(name) ASC",
                    [ctx.workspace_id.0.into(), kid.into()],
                ))
                .all(&self.conn)
                .await
                .map_err(db_err)?,
            _ => vec![],
        };

        rows.into_iter().map(task_view_from).collect()
    }

    async fn update(
        &self,
        ctx: &WorkspaceCtx,
        id: TaskViewId,
        name: String,
        filters: TaskViewFilters,
    ) -> Result<TaskView, DomainError> {
        let row = self.find_raw(ctx, id).await?.ok_or(DomainError::NotFound {
            entity: "task_view",
            id: id.0,
        })?;

        let filters_value = serde_json::to_value(&filters).map_err(|e| DomainError::Internal {
            message: format!("failed to serialize task_view filters: {e}"),
        })?;

        let mut active = row.into_active_model();
        active.name = Set(name);
        active.filters = Set(filters_value);
        active.updated_at = Set(Utc::now());

        match active.update(&self.conn).await {
            Ok(updated) => task_view_from(updated),
            Err(e) => match e.sql_err() {
                Some(SqlErr::UniqueConstraintViolation(_)) => Err(DomainError::AlreadyExists {
                    message: "a task view with this name already exists".to_string(),
                }),
                _ => Err(db_err(e)),
            },
        }
    }

    async fn delete(&self, ctx: &WorkspaceCtx, id: TaskViewId) -> Result<(), DomainError> {
        let row = self.find_raw(ctx, id).await?.ok_or(DomainError::NotFound {
            entity: "task_view",
            id: id.0,
        })?;

        let mut active = row.into_active_model();
        active.deleted_at = Set(Some(Utc::now()));

        active.update(&self.conn).await.map_err(db_err)?;

        Ok(())
    }
}

impl PgTaskViewRepo {
    /// Fetches the raw SeaORM model after verifying workspace scope and owner.
    ///
    /// Returns `None` for missing rows, soft-deleted rows, wrong-workspace rows, and
    /// rows owned by a different principal — all are concealed identically to prevent
    /// IDOR information leakage.
    async fn find_raw(
        &self,
        ctx: &WorkspaceCtx,
        id: TaskViewId,
    ) -> Result<Option<task_view::Model>, DomainError> {
        let model = task_view::Entity::find_by_id(id.0)
            .filter(task_view::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task_view::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?;

        let Some(m) = model else {
            return Ok(None);
        };

        let tv = task_view_from(m.clone())?;
        if !tv.owner.matches_actor(&ctx.actor) {
            return Ok(None);
        }

        Ok(Some(m))
    }

    async fn count_live(&self, ctx: &WorkspaceCtx) -> Result<i64, DomainError> {
        let (owner_user_col, owner_key_col) = owner_columns(&ctx.actor);

        let row = match (owner_user_col, owner_key_col) {
            (Some(uid), None) => self
                .conn
                .query_one_raw(Statement::from_sql_and_values(
                    sea_orm::DatabaseBackend::Postgres,
                    "SELECT count(*) AS n FROM task_views \
                         WHERE workspace_id = $1 AND owner_user_id = $2 AND deleted_at IS NULL",
                    [ctx.workspace_id.0.into(), uid.into()],
                ))
                .await
                .map_err(db_err)?,
            (None, Some(kid)) => self
                .conn
                .query_one_raw(Statement::from_sql_and_values(
                    sea_orm::DatabaseBackend::Postgres,
                    "SELECT count(*) AS n FROM task_views \
                         WHERE workspace_id = $1 AND owner_api_key_id = $2 AND deleted_at IS NULL",
                    [ctx.workspace_id.0.into(), kid.into()],
                ))
                .await
                .map_err(db_err)?,
            _ => return Ok(0),
        };

        let n: i64 = row
            .ok_or_else(|| DomainError::Internal {
                message: "count query returned no rows".to_string(),
            })?
            .try_get_by_index(0)
            .map_err(|e| DomainError::Internal {
                message: e.to_string(),
            })?;

        Ok(n)
    }
}

/// Returns `(owner_user_id, owner_api_key_id)` for the XOR owner columns.
fn owner_columns(actor: &atlas_domain::Actor) -> (Option<uuid::Uuid>, Option<uuid::Uuid>) {
    match actor {
        atlas_domain::Actor::User(uid) => (Some(uid.0), None),
        atlas_domain::Actor::ApiKey(kid) => (None, Some(kid.0)),
    }
}

fn db_err(e: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: e.to_string(),
    }
}
