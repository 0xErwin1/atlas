use crate::persistence::entities::comments::{
    comment, comment_from, comment_link, comment_link_event, comment_link_from,
};
use async_trait::async_trait;
use atlas_domain::{
    Actor, DomainError, WorkspaceCtx,
    entities::comments::{
        CommentFeedCursor, CommentFeedEntry, CommentLink, CommentLinkEvent, CommentLinkEventKind,
        CommentLinkTarget, CommentOwner,
    },
    ids::{ApiKeyId, CommentId, CommentLinkEventId, DocumentId, TaskId, UserId},
    ports::comments::CommentLinkRepo,
    wikilink::{CommentAttachmentUrlOwner, CommentLinkCandidate},
};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, QueryFilter, QueryOrder, Statement, TransactionTrait,
};

pub struct PgCommentLinkRepo {
    conn: DatabaseConnection,
}

impl PgCommentLinkRepo {
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    pub async fn replace_for_comment_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        comment_id: CommentId,
        targets: Vec<CommentLinkTarget>,
    ) -> Result<(), DomainError> {
        let comment = scoped_comment(conn, ctx, comment_id).await?;
        let owner = owner_from_comment(&comment)?;
        let existing = comment_link::Entity::find()
            .filter(comment_link::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(comment_link::Column::CommentId.eq(comment_id.0))
            .all(conn)
            .await
            .map_err(db_err)?;
        let desired = targets.into_iter().fold(Vec::new(), |mut unique, target| {
            if !unique.contains(&target) {
                unique.push(target);
            }
            unique
        });
        let current = existing
            .iter()
            .cloned()
            .map(comment_link_from)
            .collect::<Vec<_>>();

        for link in current
            .iter()
            .filter(|link| !desired.contains(&link.target))
        {
            comment_link::Entity::delete_by_id(link.id.0)
                .exec(conn)
                .await
                .map_err(db_err)?;
            insert_event(
                conn,
                ctx,
                owner,
                comment_id,
                CommentLinkEventKind::LinkRemoved,
                Some(link.target),
            )
            .await?;
        }
        for target in desired
            .into_iter()
            .filter(|target| !current.iter().any(|link| link.target == *target))
        {
            let (document, task, attachment) = target_columns(target);
            comment_link::ActiveModel {
                id: Set(uuid::Uuid::now_v7()),
                workspace_id: Set(ctx.workspace_id.0),
                comment_id: Set(comment_id.0),
                target_document_id: Set(document),
                target_task_id: Set(task),
                target_attachment_id: Set(attachment),
                created_at: Set(Utc::now()),
            }
            .insert(conn)
            .await
            .map_err(db_err)?;
            insert_event(
                conn,
                ctx,
                owner,
                comment_id,
                CommentLinkEventKind::LinkAdded,
                Some(target),
            )
            .await?;
        }
        Ok(())
    }

    pub async fn remove_for_comment_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        comment_id: CommentId,
    ) -> Result<(), DomainError> {
        Self::replace_for_comment_in(conn, ctx, comment_id, Vec::new()).await
    }

    pub async fn classify_candidates_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        candidates: Vec<CommentLinkCandidate>,
    ) -> Result<Vec<CommentLinkTarget>, DomainError> {
        classify_candidates(conn, ctx, candidates).await
    }

    pub async fn record_comment_deleted_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        comment_id: CommentId,
    ) -> Result<(), DomainError> {
        let comment = scoped_comment(conn, ctx, comment_id).await?;
        insert_event(
            conn,
            ctx,
            owner_from_comment(&comment)?,
            comment_id,
            CommentLinkEventKind::CommentDeleted,
            None,
        )
        .await
    }
}

#[async_trait]
impl CommentLinkRepo for PgCommentLinkRepo {
    async fn replace_for_comment(
        &self,
        ctx: &WorkspaceCtx,
        comment_id: CommentId,
        targets: Vec<CommentLinkTarget>,
    ) -> Result<(), DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;
        Self::replace_for_comment_in(&txn, ctx, comment_id, targets).await?;
        txn.commit().await.map_err(db_err)
    }
    async fn remove_for_comment(
        &self,
        ctx: &WorkspaceCtx,
        comment_id: CommentId,
    ) -> Result<(), DomainError> {
        self.replace_for_comment(ctx, comment_id, Vec::new()).await
    }
    async fn backlinks_for_target(
        &self,
        ctx: &WorkspaceCtx,
        target: CommentLinkTarget,
    ) -> Result<Vec<CommentLink>, DomainError> {
        comment_link::Entity::find()
            .filter(comment_link::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(target_condition(target))
            .order_by_asc(comment_link::Column::CreatedAt)
            .all(&self.conn)
            .await
            .map_err(db_err)
            .map(|rows| rows.into_iter().map(comment_link_from).collect())
    }
    async fn links_for_comments(
        &self,
        ctx: &WorkspaceCtx,
        ids: &[CommentId],
    ) -> Result<Vec<CommentLink>, DomainError> {
        let ids = ids.iter().map(|id| id.0).collect::<Vec<_>>();
        comment_link::Entity::find()
            .filter(comment_link::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(comment_link::Column::CommentId.is_in(ids))
            .all(&self.conn)
            .await
            .map_err(db_err)
            .map(|rows| rows.into_iter().map(comment_link_from).collect())
    }
    async fn feed_for_owner(
        &self,
        ctx: &WorkspaceCtx,
        owner: CommentOwner,
        after: Option<CommentFeedCursor>,
        limit: u64,
    ) -> Result<Vec<CommentFeedEntry>, DomainError> {
        let (comments, events) = tokio::try_join!(
            comment::Entity::find()
                .filter(comment::Column::WorkspaceId.eq(ctx.workspace_id.0))
                .filter(owner_condition(owner))
                .filter(comment::Column::DeletedAt.is_null())
                .all(&self.conn),
            comment_link_event::Entity::find()
                .filter(comment_link_event::Column::WorkspaceId.eq(ctx.workspace_id.0))
                .filter(event_owner_condition(owner))
                .all(&self.conn),
        )
        .map_err(db_err)?;
        let mut entries = comments
            .into_iter()
            .map(|row| CommentFeedEntry::Comment(comment_from(row)))
            .collect::<Vec<_>>();
        entries.extend(
            events
                .into_iter()
                .map(event_from)
                .collect::<Result<Vec<_>, _>>()?,
        );
        entries.sort_by_key(|entry| entry.cursor());
        Ok(entries
            .into_iter()
            .filter(|entry| after.is_none_or(|cursor| entry.cursor() > cursor))
            .take(limit as usize)
            .collect())
    }
}

#[async_trait]
impl atlas_domain::ports::comments::CommentLinkTargetRepo for PgCommentLinkRepo {
    async fn classify_candidates(
        &self,
        ctx: &WorkspaceCtx,
        candidates: Vec<CommentLinkCandidate>,
    ) -> Result<Vec<CommentLinkTarget>, DomainError> {
        classify_candidates(&self.conn, ctx, candidates).await
    }
}

async fn classify_candidates(
    conn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
    candidates: Vec<CommentLinkCandidate>,
) -> Result<Vec<CommentLinkTarget>, DomainError> {
    let mut targets = Vec::new();

    for candidate in candidates {
        match candidate {
            CommentLinkCandidate::Uuid(id) => {
                if crate::persistence::entities::documents::document::Entity::find_by_id(id)
                    .filter(
                        crate::persistence::entities::documents::document::Column::WorkspaceId
                            .eq(ctx.workspace_id.0),
                    )
                    .filter(
                        crate::persistence::entities::documents::document::Column::DeletedAt
                            .is_null(),
                    )
                    .one(conn)
                    .await
                    .map_err(db_err)?
                    .is_some()
                {
                    targets.push(CommentLinkTarget::Document(DocumentId(id)));
                } else if crate::persistence::entities::boards_tasks::task::Entity::find_by_id(id)
                    .filter(
                        crate::persistence::entities::boards_tasks::task::Column::WorkspaceId
                            .eq(ctx.workspace_id.0),
                    )
                    .filter(
                        crate::persistence::entities::boards_tasks::task::Column::DeletedAt
                            .is_null(),
                    )
                    .one(conn)
                    .await
                    .map_err(db_err)?
                    .is_some()
                {
                    targets.push(CommentLinkTarget::Task(TaskId(id)));
                }
            }
            CommentLinkCandidate::AttachmentUrl(url) => {
                if attachment_url_matches_owner(conn, ctx, &url).await? {
                    targets.push(CommentLinkTarget::Attachment(
                        atlas_domain::ids::AttachmentId(url.attachment_id),
                    ));
                }
            }
        }
    }

    targets.dedup();
    Ok(targets)
}

async fn attachment_url_matches_owner(
    conn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
    url: &atlas_domain::wikilink::CommentAttachmentUrl,
) -> Result<bool, DomainError> {
    let (owner_join, owner_predicate) = match &url.owner {
        CommentAttachmentUrlOwner::Task { readable_id } => (
            "JOIN tasks parent ON parent.id = comment.task_id AND parent.workspace_id = attachment.workspace_id AND parent.deleted_at IS NULL",
            ("parent.readable_id", readable_id),
        ),
        CommentAttachmentUrlOwner::Document { slug } => (
            "JOIN documents parent ON parent.id = comment.document_id AND parent.workspace_id = attachment.workspace_id AND parent.deleted_at IS NULL",
            ("parent.slug", slug),
        ),
    };
    let statement = Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        format!(
            "SELECT EXISTS(
                SELECT 1
                FROM attachments attachment
                JOIN comments comment ON comment.id = attachment.comment_id
                  AND comment.workspace_id = attachment.workspace_id
                  AND comment.deleted_at IS NULL
                JOIN workspaces workspace ON workspace.id = attachment.workspace_id
                {owner_join}
                WHERE attachment.id = $1
                  AND attachment.workspace_id = $2
                  AND attachment.comment_id = $3
                  AND attachment.deleted_at IS NULL
                  AND workspace.slug = $4
                  AND {} = $5
            ) AS matches",
            owner_predicate.0
        ),
        [
            url.attachment_id.into(),
            ctx.workspace_id.0.into(),
            url.comment_id.into(),
            url.workspace_slug.clone().into(),
            owner_predicate.1.clone().into(),
        ],
    );
    let row = conn
        .query_one_raw(statement)
        .await
        .map_err(db_err)?
        .ok_or_else(|| DomainError::Internal {
            message: "attachment owner-chain query returned no row".into(),
        })?;

    row.try_get("", "matches")
        .map_err(|error| DomainError::Internal {
            message: error.to_string(),
        })
}

async fn scoped_comment(
    conn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
    id: CommentId,
) -> Result<comment::Model, DomainError> {
    comment::Entity::find_by_id(id.0)
        .filter(comment::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(comment::Column::DeletedAt.is_null())
        .one(conn)
        .await
        .map_err(db_err)?
        .ok_or(DomainError::NotFound {
            entity: "comment",
            id: id.0,
        })
}
fn owner_from_comment(row: &comment::Model) -> Result<CommentOwner, DomainError> {
    match (row.task_id, row.document_id) {
        (Some(id), None) => Ok(CommentOwner::Task(TaskId(id))),
        (None, Some(id)) => Ok(CommentOwner::Document(DocumentId(id))),
        _ => Err(DomainError::Internal {
            message: "comment owner invariant violated".into(),
        }),
    }
}
fn owner_condition(owner: CommentOwner) -> sea_orm::Condition {
    match owner {
        CommentOwner::Task(id) => sea_orm::Condition::all().add(comment::Column::TaskId.eq(id.0)),
        CommentOwner::Document(id) => {
            sea_orm::Condition::all().add(comment::Column::DocumentId.eq(id.0))
        }
    }
}
fn event_owner_condition(owner: CommentOwner) -> sea_orm::Condition {
    match owner {
        CommentOwner::Task(id) => {
            sea_orm::Condition::all().add(comment_link_event::Column::ParentTaskId.eq(id.0))
        }
        CommentOwner::Document(id) => {
            sea_orm::Condition::all().add(comment_link_event::Column::ParentDocumentId.eq(id.0))
        }
    }
}
fn target_columns(
    target: CommentLinkTarget,
) -> (Option<uuid::Uuid>, Option<uuid::Uuid>, Option<uuid::Uuid>) {
    match target {
        CommentLinkTarget::Document(id) => (Some(id.0), None, None),
        CommentLinkTarget::Task(id) => (None, Some(id.0), None),
        CommentLinkTarget::Attachment(id) => (None, None, Some(id.0)),
    }
}
fn target_condition(target: CommentLinkTarget) -> sea_orm::Condition {
    match target {
        CommentLinkTarget::Document(id) => {
            sea_orm::Condition::all().add(comment_link::Column::TargetDocumentId.eq(id.0))
        }
        CommentLinkTarget::Task(id) => {
            sea_orm::Condition::all().add(comment_link::Column::TargetTaskId.eq(id.0))
        }
        CommentLinkTarget::Attachment(id) => {
            sea_orm::Condition::all().add(comment_link::Column::TargetAttachmentId.eq(id.0))
        }
    }
}
async fn insert_event(
    conn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
    owner: CommentOwner,
    comment_id: CommentId,
    kind: CommentLinkEventKind,
    target: Option<CommentLinkTarget>,
) -> Result<(), DomainError> {
    let (parent_task_id, parent_document_id) = match owner {
        CommentOwner::Task(id) => (Some(id.0), None),
        CommentOwner::Document(id) => (None, Some(id.0)),
    };
    let (document, task, attachment) = target.map(target_columns).unwrap_or((None, None, None));
    let (actor_type, actor_id) = match ctx.actor {
        Actor::User(id) => ("user".into(), id.0),
        Actor::ApiKey(id) => ("api_key".into(), id.0),
    };
    comment_link_event::ActiveModel {
        id: Set(uuid::Uuid::now_v7()),
        workspace_id: Set(ctx.workspace_id.0),
        parent_task_id: Set(parent_task_id),
        parent_document_id: Set(parent_document_id),
        comment_id: Set(comment_id.0),
        event_kind: Set(match kind {
            CommentLinkEventKind::LinkAdded => "link_added",
            CommentLinkEventKind::LinkRemoved => "link_removed",
            CommentLinkEventKind::CommentDeleted => "comment_deleted",
        }
        .into()),
        target_document_id: Set(document),
        target_task_id: Set(task),
        target_attachment_id: Set(attachment),
        actor_type: Set(actor_type),
        actor_id: Set(actor_id),
        created_at: Set(Utc::now()),
    }
    .insert(conn)
    .await
    .map_err(db_err)
    .map(|_| ())
}
fn event_from(row: comment_link_event::Model) -> Result<CommentFeedEntry, DomainError> {
    let parent = match (row.parent_task_id, row.parent_document_id) {
        (Some(id), None) => CommentOwner::Task(TaskId(id)),
        (None, Some(id)) => CommentOwner::Document(DocumentId(id)),
        _ => {
            return Err(DomainError::Internal {
                message: "comment link event parent invariant violated".into(),
            });
        }
    };
    let kind = match row.event_kind.as_str() {
        "link_added" => CommentLinkEventKind::LinkAdded,
        "link_removed" => CommentLinkEventKind::LinkRemoved,
        "comment_deleted" => CommentLinkEventKind::CommentDeleted,
        _ => {
            return Err(DomainError::Internal {
                message: "unknown comment link event kind".into(),
            });
        }
    };
    let target = match (
        row.target_document_id,
        row.target_task_id,
        row.target_attachment_id,
    ) {
        (Some(id), None, None) => Some(CommentLinkTarget::Document(DocumentId(id))),
        (None, Some(id), None) => Some(CommentLinkTarget::Task(TaskId(id))),
        (None, None, Some(id)) => Some(CommentLinkTarget::Attachment(
            atlas_domain::ids::AttachmentId(id),
        )),
        (None, None, None) => None,
        _ => {
            return Err(DomainError::Internal {
                message: "comment link event target invariant violated".into(),
            });
        }
    };
    let actor = match row.actor_type.as_str() {
        "user" => Actor::User(UserId(row.actor_id)),
        "api_key" => Actor::ApiKey(ApiKeyId(row.actor_id)),
        _ => {
            return Err(DomainError::Internal {
                message: "unknown comment link event actor".into(),
            });
        }
    };
    Ok(CommentFeedEntry::Event(CommentLinkEvent {
        id: CommentLinkEventId(row.id),
        workspace_id: atlas_domain::WorkspaceId(row.workspace_id),
        parent,
        comment_id: CommentId(row.comment_id),
        kind,
        target,
        actor,
        created_at: row.created_at,
    }))
}
fn db_err(error: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: error.to_string(),
    }
}
