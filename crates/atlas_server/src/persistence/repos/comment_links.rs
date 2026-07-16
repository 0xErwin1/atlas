use crate::persistence::entities::comments::{
    comment, comment_link, comment_link_event, comment_link_from,
};
use crate::services::CommentMutationFault;
use async_trait::async_trait;
use atlas_domain::{
    Actor, DomainError, WorkspaceCtx,
    entities::comments::{
        CommentBacklink, CommentFeedCursor, CommentFeedEntry, CommentFeedPage, CommentLink,
        CommentLinkEvent, CommentLinkEventKind, CommentLinkTarget, CommentOwner,
    },
    ids::{ApiKeyId, CommentId, CommentLinkEventId, DocumentId, TaskId, UserId},
    ports::comments::CommentLinkRepo,
    wikilink::{CommentAttachmentUrlOwner, CommentLinkCandidate},
};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, QueryFilter, Statement, TransactionTrait,
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
        Self::replace_for_comment_with_fault_in(conn, ctx, comment_id, targets, None).await
    }

    pub(crate) async fn replace_for_comment_with_fault_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        comment_id: CommentId,
        targets: Vec<CommentLinkTarget>,
        fault: Option<CommentMutationFault>,
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

        let removed = current
            .iter()
            .filter(|link| !desired.contains(&link.target))
            .cloned()
            .collect::<Vec<_>>();
        let added = desired
            .into_iter()
            .filter(|target| !current.iter().any(|link| link.target == *target))
            .collect::<Vec<_>>();

        for link in &removed {
            comment_link::Entity::delete_by_id(link.id.0)
                .exec(conn)
                .await
                .map_err(db_err)?;
        }
        for target in &added {
            let (document, task, attachment) = target_columns(*target);
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
        }

        if fault == Some(CommentMutationFault::AfterGraphReplace) {
            return Err(injected_fault(CommentMutationFault::AfterGraphReplace));
        }

        for link in removed {
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
        for target in added {
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

        if fault == Some(CommentMutationFault::AfterEventAppend) {
            return Err(injected_fault(CommentMutationFault::AfterEventAppend));
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
    ) -> Result<Vec<CommentBacklink>, DomainError> {
        let (target_column, target_id) = match target {
            CommentLinkTarget::Document(id) => ("target_document_id", id.0),
            CommentLinkTarget::Task(id) => ("target_task_id", id.0),
            CommentLinkTarget::Attachment(_) => return Ok(Vec::new()),
        };
        let rows = self
            .conn
            .query_all_raw(Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                format!(
                    "SELECT cl.id, cl.workspace_id, cl.comment_id, cl.target_document_id, cl.target_task_id, \
                     cl.target_attachment_id, cl.created_at, c.task_id AS parent_task_id, \
                     c.document_id AS parent_document_id, parent_task.readable_id AS parent_readable_id, \
                     parent_document.slug AS parent_slug, \
                     COALESCE(parent_task.title, parent_document.title) AS parent_title \
                     FROM comment_links cl \
                     JOIN comments c ON c.id = cl.comment_id AND c.workspace_id = cl.workspace_id \
                     LEFT JOIN tasks parent_task ON parent_task.id = c.task_id AND parent_task.workspace_id = c.workspace_id AND parent_task.deleted_at IS NULL \
                     LEFT JOIN documents parent_document ON parent_document.id = c.document_id AND parent_document.workspace_id = c.workspace_id AND parent_document.deleted_at IS NULL \
                      WHERE cl.workspace_id = $1 AND cl.{target_column} = $2 AND c.deleted_at IS NULL \
                        AND ((c.task_id IS NOT NULL AND parent_task.id IS NOT NULL) \
                          OR (c.document_id IS NOT NULL AND parent_document.id IS NOT NULL)) \
                      ORDER BY cl.created_at ASC, cl.id ASC"
                ),
                [ctx.workspace_id.0.into(), target_id.into()],
            ))
            .await
            .map_err(db_err)?;

        rows.into_iter().map(comment_backlink_from_row).collect()
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
    ) -> Result<CommentFeedPage, DomainError> {
        let (parent_column, parent_id) = match owner {
            CommentOwner::Task(id) => ("task_id", id.0),
            CommentOwner::Document(id) => ("document_id", id.0),
        };
        let (cursor_predicate, mut values, limit_parameter) = match after {
            Some(cursor) => (
                " AND (created_at, id) > ($3, $4)",
                vec![
                    ctx.workspace_id.0.into(),
                    parent_id.into(),
                    cursor.created_at.into(),
                    cursor.id.into(),
                ],
                "$5",
            ),
            None => ("", vec![ctx.workspace_id.0.into(), parent_id.into()], "$3"),
        };
        values.push((limit.saturating_add(1) as i64).into());

        let statement = Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            format!(
                "WITH feed AS (\
                    SELECT id, id AS comment_id, workspace_id, task_id AS parent_task_id, document_id AS parent_document_id, \
                           body, created_by_user_id, created_by_api_key_id, created_at, updated_at, \
                           NULL::text AS event_kind, NULL::uuid AS target_document_id, \
                           NULL::uuid AS target_task_id, NULL::uuid AS target_attachment_id, \
                           NULL::text AS actor_type, NULL::uuid AS actor_id, 'comment'::text AS entry_type \
                    FROM comments \
                    WHERE workspace_id = $1 AND {parent_column} = $2 AND deleted_at IS NULL{cursor_predicate} \
                    UNION ALL \
                    SELECT id, comment_id, workspace_id, parent_task_id, parent_document_id, \
                           NULL::text AS body, NULL::uuid AS created_by_user_id, \
                           NULL::uuid AS created_by_api_key_id, created_at, created_at AS updated_at, \
                           event_kind, target_document_id, target_task_id, target_attachment_id, \
                           actor_type, actor_id, 'event'::text AS entry_type \
                    FROM comment_link_events \
                    WHERE workspace_id = $1 AND parent_{parent_column} = $2{cursor_predicate} \
                ) \
                SELECT * FROM feed ORDER BY created_at ASC, id ASC LIMIT {limit_parameter}",
            ),
            values,
        );
        let mut entries = self
            .conn
            .query_all_raw(statement)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(feed_entry_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let has_more = entries.len() > limit as usize;
        entries.truncate(limit as usize);

        Ok(CommentFeedPage { entries, has_more })
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
fn target_columns(
    target: CommentLinkTarget,
) -> (Option<uuid::Uuid>, Option<uuid::Uuid>, Option<uuid::Uuid>) {
    match target {
        CommentLinkTarget::Document(id) => (Some(id.0), None, None),
        CommentLinkTarget::Task(id) => (None, Some(id.0), None),
        CommentLinkTarget::Attachment(id) => (None, None, Some(id.0)),
    }
}
fn comment_backlink_from_row(row: sea_orm::QueryResult) -> Result<CommentBacklink, DomainError> {
    let target = match (
        row.try_get::<Option<uuid::Uuid>>("", "target_document_id")
            .map_err(row_err)?,
        row.try_get::<Option<uuid::Uuid>>("", "target_task_id")
            .map_err(row_err)?,
        row.try_get::<Option<uuid::Uuid>>("", "target_attachment_id")
            .map_err(row_err)?,
    ) {
        (Some(id), None, None) => CommentLinkTarget::Document(DocumentId(id)),
        (None, Some(id), None) => CommentLinkTarget::Task(TaskId(id)),
        (None, None, Some(id)) => {
            CommentLinkTarget::Attachment(atlas_domain::ids::AttachmentId(id))
        }
        _ => {
            return Err(DomainError::Internal {
                message: "comment backlink target invariant violated".into(),
            });
        }
    };
    let parent = match (
        row.try_get::<Option<uuid::Uuid>>("", "parent_task_id")
            .map_err(row_err)?,
        row.try_get::<Option<uuid::Uuid>>("", "parent_document_id")
            .map_err(row_err)?,
    ) {
        (Some(id), None) => CommentOwner::Task(TaskId(id)),
        (None, Some(id)) => CommentOwner::Document(DocumentId(id)),
        _ => {
            return Err(DomainError::Internal {
                message: "comment backlink parent invariant violated".into(),
            });
        }
    };
    Ok(CommentBacklink {
        id: atlas_domain::ids::CommentLinkId(row.try_get("", "id").map_err(row_err)?),
        workspace_id: atlas_domain::WorkspaceId(row.try_get("", "workspace_id").map_err(row_err)?),
        comment_id: CommentId(row.try_get("", "comment_id").map_err(row_err)?),
        parent,
        parent_readable_id: row.try_get("", "parent_readable_id").map_err(row_err)?,
        parent_slug: row.try_get("", "parent_slug").map_err(row_err)?,
        parent_title: row.try_get("", "parent_title").map_err(row_err)?,
        target,
        created_at: row.try_get("", "created_at").map_err(row_err)?,
    })
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
fn feed_entry_from_row(row: sea_orm::QueryResult) -> Result<CommentFeedEntry, DomainError> {
    let entry_type = row.try_get::<String>("", "entry_type").map_err(row_err)?;
    let id = row.try_get("", "id").map_err(row_err)?;
    let workspace_id = row.try_get("", "workspace_id").map_err(row_err)?;
    let created_at = row.try_get("", "created_at").map_err(row_err)?;

    if entry_type == "comment" {
        return Ok(CommentFeedEntry::Comment(
            atlas_domain::entities::comments::Comment {
                id: CommentId(id),
                workspace_id: atlas_domain::WorkspaceId(workspace_id),
                task_id: row
                    .try_get::<Option<uuid::Uuid>>("", "parent_task_id")
                    .map_err(row_err)?
                    .map(TaskId),
                document_id: row
                    .try_get::<Option<uuid::Uuid>>("", "parent_document_id")
                    .map_err(row_err)?
                    .map(DocumentId),
                body: row.try_get("", "body").map_err(row_err)?,
                created_by: crate::persistence::entities::boards_tasks::actor_from_columns(
                    row.try_get("", "created_by_user_id").map_err(row_err)?,
                    row.try_get("", "created_by_api_key_id").map_err(row_err)?,
                ),
                created_at,
                updated_at: row.try_get("", "updated_at").map_err(row_err)?,
                deleted_at: None,
            },
        ));
    }

    if entry_type != "event" {
        return Err(DomainError::Internal {
            message: "unknown comment feed entry type".into(),
        });
    }

    let parent = match (
        row.try_get::<Option<uuid::Uuid>>("", "parent_task_id")
            .map_err(row_err)?,
        row.try_get::<Option<uuid::Uuid>>("", "parent_document_id")
            .map_err(row_err)?,
    ) {
        (Some(id), None) => CommentOwner::Task(TaskId(id)),
        (None, Some(id)) => CommentOwner::Document(DocumentId(id)),
        _ => {
            return Err(DomainError::Internal {
                message: "comment link event parent invariant violated".into(),
            });
        }
    };
    let event_kind = row.try_get::<String>("", "event_kind").map_err(row_err)?;
    let kind = match event_kind.as_str() {
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
        row.try_get::<Option<uuid::Uuid>>("", "target_document_id")
            .map_err(row_err)?,
        row.try_get::<Option<uuid::Uuid>>("", "target_task_id")
            .map_err(row_err)?,
        row.try_get::<Option<uuid::Uuid>>("", "target_attachment_id")
            .map_err(row_err)?,
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
    let actor_type = row.try_get::<String>("", "actor_type").map_err(row_err)?;
    let actor_id = row.try_get::<uuid::Uuid>("", "actor_id").map_err(row_err)?;
    let actor = match actor_type.as_str() {
        "user" => Actor::User(UserId(actor_id)),
        "api_key" => Actor::ApiKey(ApiKeyId(actor_id)),
        _ => {
            return Err(DomainError::Internal {
                message: "unknown comment link event actor".into(),
            });
        }
    };
    Ok(CommentFeedEntry::Event(CommentLinkEvent {
        id: CommentLinkEventId(id),
        workspace_id: atlas_domain::WorkspaceId(workspace_id),
        parent,
        comment_id: CommentId(row.try_get("", "comment_id").map_err(row_err)?),
        kind,
        target,
        actor,
        created_at,
    }))
}

fn row_err(error: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: error.to_string(),
    }
}
fn db_err(error: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: error.to_string(),
    }
}

fn injected_fault(point: CommentMutationFault) -> DomainError {
    DomainError::Internal {
        message: format!("injected comment mutation fault at {point:?}"),
    }
}
