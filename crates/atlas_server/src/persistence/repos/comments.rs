use async_trait::async_trait;
use atlas_domain::{
    Actor, DomainError, WorkspaceCtx,
    entities::comments::{Comment, CommentOwner, NewComment},
    ids::CommentId,
};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait,
    DatabaseConnection, EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, QuerySelect,
    sea_query::Expr,
};

use crate::persistence::entities::{
    comments::{comment, comment_from},
    documents::attachment,
};
use crate::persistence::live_ancestors::{live_document_chain, live_task_chain};

pub use atlas_domain::ports::comments::CommentRepo;

pub struct PgCommentRepo {
    pub conn: DatabaseConnection,
}

impl PgCommentRepo {
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Inserts a comment inside an existing transaction.
    pub async fn create_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        new: NewComment,
    ) -> Result<Comment, DomainError> {
        Self::create_with_id_in(conn, ctx, new, CommentId::new()).await
    }

    pub async fn create_with_id_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        new: NewComment,
        id: CommentId,
    ) -> Result<Comment, DomainError> {
        let (task_id, document_id) = owner_columns(new.owner);
        let (created_by_user_id, created_by_api_key_id) = actor_columns(&ctx.actor);
        let now = Utc::now();

        let model = comment::ActiveModel {
            id: Set(id.0),
            workspace_id: Set(ctx.workspace_id.0),
            task_id: Set(task_id),
            document_id: Set(document_id),
            body: Set(new.body),
            created_by_user_id: Set(created_by_user_id),
            created_by_api_key_id: Set(created_by_api_key_id),
            created_at: Set(now),
            updated_at: Set(now),
            deleted_at: Set(None),
        };

        model.insert(conn).await.map(comment_from).map_err(db_err)
    }

    /// Fetches a single comment scoped to `owner`, inside an existing transaction.
    ///
    /// `owner` scopes the lookup so a comment id from a different task/document
    /// resolves to `NotFound` — this is the IDOR guard for cross-owner ids.
    pub async fn get_for_owner_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        owner: CommentOwner,
        id: CommentId,
    ) -> Result<Comment, DomainError> {
        find_scoped(conn, ctx, owner, id).await.map(comment_from)
    }

    /// Fetches a live comment while holding its row lock for a lifecycle mutation.
    pub async fn get_for_owner_for_update_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        owner: CommentOwner,
        id: CommentId,
    ) -> Result<Comment, DomainError> {
        find_scoped_for_update(conn, ctx, owner, id)
            .await
            .map(comment_from)
    }

    /// Updates a comment's body and `updated_at` inside an existing transaction,
    /// given the caller's already-loaded copy of the row (typically from
    /// `get_for_owner_in`), returning the updated row.
    ///
    /// Re-applies the same scoping as `find_scoped` (workspace, owner,
    /// `deleted_at IS NULL`) to the update itself instead of loading the row a
    /// second time, so a concurrent soft-delete between the caller's load and
    /// this update still resolves to `NotFound` instead of reviving the row.
    pub async fn update_body_from(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        owner: CommentOwner,
        loaded: Comment,
        new_body: String,
    ) -> Result<Comment, DomainError> {
        let now = Utc::now();

        let result = comment::Entity::update_many()
            .col_expr(comment::Column::Body, Expr::value(new_body.clone()))
            .col_expr(comment::Column::UpdatedAt, Expr::value(now))
            .filter(comment::Column::Id.eq(loaded.id.0))
            .filter(comment::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(owner_condition(owner))
            .filter(comment::Column::DeletedAt.is_null())
            .exec(conn)
            .await
            .map_err(db_err)?;

        if result.rows_affected == 0 {
            return Err(DomainError::NotFound {
                entity: "comment",
                id: loaded.id.0,
            });
        }

        Ok(Comment {
            body: new_body,
            updated_at: now,
            ..loaded
        })
    }

    /// Soft-deletes a comment inside an existing transaction.
    ///
    /// `owner` scopes the lookup so a comment id from a different task/document
    /// resolves to `NotFound` — this is the IDOR guard for cross-owner ids.
    pub async fn soft_delete_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        owner: CommentOwner,
        id: CommentId,
    ) -> Result<(), DomainError> {
        let row = find_scoped(conn, ctx, owner, id).await?;

        let mut active = row.into_active_model();
        active.deleted_at = Set(Some(Utc::now()));
        active.updated_at = Set(Utc::now());
        active.update(conn).await.map_err(db_err)?;
        Ok(())
    }

    /// Tombstones a locked live comment at the supplied lifecycle timestamp.
    pub async fn soft_delete_at_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        owner: CommentOwner,
        id: CommentId,
        deleted_at: chrono::DateTime<Utc>,
    ) -> Result<(), DomainError> {
        let result = comment::Entity::update_many()
            .col_expr(comment::Column::DeletedAt, Expr::value(deleted_at))
            .col_expr(comment::Column::UpdatedAt, Expr::value(deleted_at))
            .filter(comment::Column::Id.eq(id.0))
            .filter(comment::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(owner_condition(owner))
            .filter(comment::Column::DeletedAt.is_null())
            .exec(conn)
            .await
            .map_err(db_err)?;

        if result.rows_affected == 0 {
            return Err(DomainError::NotFound {
                entity: "comment",
                id: id.0,
            });
        }

        Ok(())
    }

    /// Restores a comment and only attachments tombstoned by the same delete operation.
    pub async fn restore_at_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        owner: CommentOwner,
        id: CommentId,
        deleted_at: chrono::DateTime<Utc>,
    ) -> Result<(), DomainError> {
        let row = comment::Entity::find_by_id(id.0)
            .filter(comment::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(owner_condition(owner))
            .filter(comment::Column::DeletedAt.eq(deleted_at))
            .lock_exclusive()
            .one(conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "comment",
                id: id.0,
            })?;

        let mut active = row.into_active_model();
        active.deleted_at = Set(None);
        active.updated_at = Set(Utc::now());
        active.update(conn).await.map_err(db_err)?;

        attachment::Entity::update_many()
            .col_expr(
                attachment::Column::DeletedAt,
                Expr::value(None::<chrono::DateTime<Utc>>),
            )
            .col_expr(attachment::Column::UpdatedAt, Expr::value(Utc::now()))
            .filter(attachment::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(attachment::Column::CommentId.eq(id.0))
            .filter(attachment::Column::DeletedAt.eq(deleted_at))
            .exec(conn)
            .await
            .map_err(db_err)?;

        Ok(())
    }
}

#[async_trait]
impl CommentRepo for PgCommentRepo {
    async fn create(&self, ctx: &WorkspaceCtx, new: NewComment) -> Result<Comment, DomainError> {
        PgCommentRepo::create_in(&self.conn, ctx, new).await
    }

    async fn get_for_owner(
        &self,
        ctx: &WorkspaceCtx,
        owner: CommentOwner,
        id: CommentId,
    ) -> Result<Comment, DomainError> {
        PgCommentRepo::get_for_owner_in(&self.conn, ctx, owner, id).await
    }

    /// Lists comments oldest-first, id-cursor paginated.
    ///
    /// Deliberately the inverse of `TaskActivityRepo::list_for_task`'s newest-first
    /// `order_by_desc` + `Id.lt(cursor)`: comments read as a conversation, so the
    /// oldest entry comes first and paging walks forward with `Id.gt(cursor)`.
    async fn list_for_owner(
        &self,
        ctx: &WorkspaceCtx,
        owner: CommentOwner,
        after_id: Option<CommentId>,
        limit: u64,
    ) -> Result<Vec<Comment>, DomainError> {
        let mut q = comment::Entity::find()
            .filter(comment::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(owner_condition(owner))
            .filter(comment::Column::DeletedAt.is_null())
            .filter(live_task_chain("comments.task_id"))
            .filter(live_document_chain("comments.document_id"))
            .order_by_asc(comment::Column::Id)
            .limit(limit);

        if let Some(cursor) = after_id {
            q = q.filter(comment::Column::Id.gt(cursor.0));
        }

        q.all(&self.conn)
            .await
            .map(|rows| rows.into_iter().map(comment_from).collect())
            .map_err(db_err)
    }

    async fn soft_delete(
        &self,
        ctx: &WorkspaceCtx,
        owner: CommentOwner,
        id: CommentId,
    ) -> Result<(), DomainError> {
        PgCommentRepo::soft_delete_in(&self.conn, ctx, owner, id).await
    }
}

async fn find_scoped(
    conn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
    owner: CommentOwner,
    id: CommentId,
) -> Result<comment::Model, DomainError> {
    comment::Entity::find_by_id(id.0)
        .filter(comment::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(owner_condition(owner))
        .filter(comment::Column::DeletedAt.is_null())
        .filter(live_task_chain("comments.task_id"))
        .filter(live_document_chain("comments.document_id"))
        .one(conn)
        .await
        .map_err(db_err)?
        .ok_or(DomainError::NotFound {
            entity: "comment",
            id: id.0,
        })
}

async fn find_scoped_for_update(
    conn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
    owner: CommentOwner,
    id: CommentId,
) -> Result<comment::Model, DomainError> {
    comment::Entity::find_by_id(id.0)
        .filter(comment::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(owner_condition(owner))
        .filter(comment::Column::DeletedAt.is_null())
        .filter(live_task_chain("comments.task_id"))
        .filter(live_document_chain("comments.document_id"))
        .lock_exclusive()
        .one(conn)
        .await
        .map_err(db_err)?
        .ok_or(DomainError::NotFound {
            entity: "comment",
            id: id.0,
        })
}

fn owner_condition(owner: CommentOwner) -> Condition {
    match owner {
        CommentOwner::Task(id) => Condition::all().add(comment::Column::TaskId.eq(id.0)),
        CommentOwner::Document(id) => Condition::all().add(comment::Column::DocumentId.eq(id.0)),
    }
}

fn owner_columns(owner: CommentOwner) -> (Option<uuid::Uuid>, Option<uuid::Uuid>) {
    match owner {
        CommentOwner::Task(id) => (Some(id.0), None),
        CommentOwner::Document(id) => (None, Some(id.0)),
    }
}

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
