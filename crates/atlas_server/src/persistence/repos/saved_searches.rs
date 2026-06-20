use async_trait::async_trait;
use atlas_domain::{
    DomainError, SavedSearchId, WorkspaceCtx,
    entities::saved_searches::{NewSavedSearch, SavedSearch},
};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, IntoActiveModel, QueryFilter, SqlErr, Statement,
};

use crate::persistence::entities::saved_searches::{saved_search, saved_search_from};

pub use atlas_domain::ports::saved_searches::SavedSearchRepo;

pub struct PgSavedSearchRepo {
    pub conn: DatabaseConnection,
}

impl PgSavedSearchRepo {
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl SavedSearchRepo for PgSavedSearchRepo {
    /// Creates a new saved search for the calling principal.
    ///
    /// The COUNT-before-INSERT cap check at >= 100 is not transactional, so two
    /// concurrent creates at n=99 can both pass and briefly exceed the cap by one.
    /// This is accepted: the cap is an abuse ceiling, not a hard invariant, and
    /// an off-by-one under concurrency is harmless.
    async fn create(
        &self,
        ctx: &WorkspaceCtx,
        new: NewSavedSearch,
    ) -> Result<SavedSearch, DomainError> {
        let (owner_user_col, owner_key_col) = owner_columns(&ctx.actor);

        let count = self.count_live(ctx).await?;
        if count >= 100 {
            return Err(DomainError::InvalidInput {
                message: "you have reached the maximum of 100 saved searches for this workspace; \
                          delete one before creating another"
                    .to_string(),
            });
        }

        let model = saved_search::ActiveModel {
            id: Set(SavedSearchId::new().0),
            workspace_id: Set(ctx.workspace_id.0),
            name: Set(new.name),
            query: Set(new.query),
            owner_user_id: Set(owner_user_col),
            owner_api_key_id: Set(owner_key_col),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            deleted_at: Set(None),
        };

        match model.insert(&self.conn).await {
            Ok(inserted) => Ok(saved_search_from(inserted)),
            Err(e) => match e.sql_err() {
                Some(SqlErr::UniqueConstraintViolation(_)) => Err(DomainError::AlreadyExists {
                    message: "a saved search with this name already exists".to_string(),
                }),
                _ => Err(db_err(e)),
            },
        }
    }

    async fn find(
        &self,
        ctx: &WorkspaceCtx,
        id: SavedSearchId,
    ) -> Result<Option<SavedSearch>, DomainError> {
        let model = self.find_raw(ctx, id).await?;
        Ok(model.map(saved_search_from))
    }

    async fn list_for_owner(&self, ctx: &WorkspaceCtx) -> Result<Vec<SavedSearch>, DomainError> {
        let (owner_user_col, owner_key_col) = owner_columns(&ctx.actor);

        let rows = match (owner_user_col, owner_key_col) {
            (Some(uid), None) => saved_search::Entity::find()
                .from_raw_sql(Statement::from_sql_and_values(
                    sea_orm::DatabaseBackend::Postgres,
                    "SELECT id, workspace_id, name, query, owner_user_id, owner_api_key_id, \
                         created_at, updated_at, deleted_at \
                         FROM saved_searches \
                         WHERE workspace_id = $1 AND owner_user_id = $2 AND deleted_at IS NULL \
                         ORDER BY lower(name) ASC",
                    [ctx.workspace_id.0.into(), uid.into()],
                ))
                .all(&self.conn)
                .await
                .map_err(db_err)?,
            (None, Some(kid)) => saved_search::Entity::find()
                .from_raw_sql(Statement::from_sql_and_values(
                    sea_orm::DatabaseBackend::Postgres,
                    "SELECT id, workspace_id, name, query, owner_user_id, owner_api_key_id, \
                         created_at, updated_at, deleted_at \
                         FROM saved_searches \
                         WHERE workspace_id = $1 AND owner_api_key_id = $2 AND deleted_at IS NULL \
                         ORDER BY lower(name) ASC",
                    [ctx.workspace_id.0.into(), kid.into()],
                ))
                .all(&self.conn)
                .await
                .map_err(db_err)?,
            _ => vec![],
        };

        Ok(rows.into_iter().map(saved_search_from).collect())
    }

    async fn rename(
        &self,
        ctx: &WorkspaceCtx,
        id: SavedSearchId,
        new_name: String,
    ) -> Result<SavedSearch, DomainError> {
        let row = self.find_raw(ctx, id).await?.ok_or(DomainError::NotFound {
            entity: "saved_search",
            id: id.0,
        })?;

        let mut active = row.into_active_model();
        active.name = Set(new_name);
        active.updated_at = Set(Utc::now());

        match active.update(&self.conn).await {
            Ok(updated) => Ok(saved_search_from(updated)),
            Err(e) => match e.sql_err() {
                Some(SqlErr::UniqueConstraintViolation(_)) => Err(DomainError::AlreadyExists {
                    message: "a saved search with this name already exists".to_string(),
                }),
                _ => Err(db_err(e)),
            },
        }
    }

    async fn delete(&self, ctx: &WorkspaceCtx, id: SavedSearchId) -> Result<(), DomainError> {
        let row = self.find_raw(ctx, id).await?.ok_or(DomainError::NotFound {
            entity: "saved_search",
            id: id.0,
        })?;

        let mut active = row.into_active_model();
        active.deleted_at = Set(Some(Utc::now()));

        active.update(&self.conn).await.map_err(db_err)?;

        Ok(())
    }
}

impl PgSavedSearchRepo {
    /// Fetches the raw SeaORM model after verifying workspace scope and owner.
    ///
    /// Returns `None` for missing rows, soft-deleted rows, wrong-workspace rows, and
    /// rows owned by a different principal — all are concealed identically.
    async fn find_raw(
        &self,
        ctx: &WorkspaceCtx,
        id: SavedSearchId,
    ) -> Result<Option<saved_search::Model>, DomainError> {
        let model = saved_search::Entity::find_by_id(id.0)
            .filter(saved_search::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(saved_search::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?;

        let Some(m) = model else {
            return Ok(None);
        };

        let ss = saved_search_from(m.clone());
        if !ss.owner.matches_actor(&ctx.actor) {
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
                    "SELECT count(*) AS n FROM saved_searches \
                         WHERE workspace_id = $1 AND owner_user_id = $2 AND deleted_at IS NULL",
                    [ctx.workspace_id.0.into(), uid.into()],
                ))
                .await
                .map_err(db_err)?,
            (None, Some(kid)) => self
                .conn
                .query_one_raw(Statement::from_sql_and_values(
                    sea_orm::DatabaseBackend::Postgres,
                    "SELECT count(*) AS n FROM saved_searches \
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
