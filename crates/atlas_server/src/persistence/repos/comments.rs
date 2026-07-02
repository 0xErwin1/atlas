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
};

use crate::persistence::entities::comments::{comment, comment_from};

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
        let (task_id, document_id) = owner_columns(new.owner);
        let (created_by_user_id, created_by_api_key_id) = actor_columns(&ctx.actor);
        let now = Utc::now();

        let model = comment::ActiveModel {
            id: Set(CommentId::new().0),
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

    /// Updates a comment's body and `updated_at` inside an existing transaction,
    /// returning the updated row.
    ///
    /// `owner` scopes the lookup so a comment id from a different task/document
    /// resolves to `NotFound` — this is the IDOR guard for cross-owner ids.
    pub async fn update_body_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        owner: CommentOwner,
        id: CommentId,
        new_body: String,
    ) -> Result<Comment, DomainError> {
        let row = find_scoped(conn, ctx, owner, id).await?;

        let mut active = row.into_active_model();
        active.body = Set(new_body);
        active.updated_at = Set(Utc::now());

        active.update(conn).await.map(comment_from).map_err(db_err)
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
