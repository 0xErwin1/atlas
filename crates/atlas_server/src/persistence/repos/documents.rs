use async_trait::async_trait;
use atlas_domain::{
    Actor, DomainError, RevisionConflict, WorkspaceCtx,
    entities::documents::{
        Attachment, AttachmentOwner, Document, DocumentLink, DocumentSummary, ExtractedLink,
        NewAttachment, NewDocument, RevisionMeta,
    },
    ids::{AttachmentId, DocumentId, FolderId, ProjectId, RevisionId},
    permissions::Principal,
    revision::{create_revision_patch, is_anchor_seq, reconstruct},
};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, TransactionTrait,
};
use serde_json::json;
use uuid::Uuid;

use crate::persistence::entities::documents::{
    attachment, attachment_from, document, document_from, document_link, document_link_from,
    document_revision, revision_meta_from,
};

pub use atlas_domain::ports::documents::{AttachmentRepo, DocumentLinkRepo, DocumentRepo};

pub struct PgDocumentRepo {
    pub conn: DatabaseConnection,
    pub anchor_interval: u32,
}

impl PgDocumentRepo {
    pub fn new(conn: DatabaseConnection, anchor_interval: u32) -> Self {
        Self {
            conn,
            anchor_interval,
        }
    }
}

#[async_trait]
impl DocumentRepo for PgDocumentRepo {
    async fn create(&self, ctx: &WorkspaceCtx, new: NewDocument) -> Result<Document, DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;

        let doc_id = DocumentId::new();
        let rev_id = RevisionId::new();
        let (by_user, by_key) = actor_fields(&ctx.actor);
        let now = Utc::now();

        let frontmatter = new.frontmatter.unwrap_or_else(|| json!({}));

        let doc_model = document::ActiveModel {
            id: Set(doc_id.0),
            workspace_id: Set(ctx.workspace_id.0),
            project_id: Set(new.project_id.map(|id| id.0)),
            folder_id: Set(new.folder_id.map(|id| id.0)),
            title: Set(new.title),
            slug: Set(new.slug),
            content: Set(new.content.clone()),
            frontmatter: Set(frontmatter),
            current_revision_id: Set(None),
            current_revision_seq: Set(0),
            created_by_user_id: Set(by_user),
            created_by_api_key_id: Set(by_key),
            created_at: Set(now),
            updated_at: Set(now),
            deleted_at: Set(None),
        };
        let inserted_doc = doc_model.insert(&txn).await.map_err(db_err)?;

        let rev_model = document_revision::ActiveModel {
            id: Set(rev_id.0),
            workspace_id: Set(ctx.workspace_id.0),
            document_id: Set(doc_id.0),
            seq: Set(1),
            patch: Set(None),
            snapshot: Set(Some(new.content.clone())),
            is_anchor: Set(true),
            created_by_user_id: Set(by_user),
            created_by_api_key_id: Set(by_key),
            created_at: Set(now),
        };
        rev_model.insert(&txn).await.map_err(db_err)?;

        let mut doc_active = inserted_doc.into_active_model();
        doc_active.current_revision_id = Set(Some(rev_id.0));
        doc_active.current_revision_seq = Set(1);
        let updated_doc = doc_active.update(&txn).await.map_err(db_err)?;

        txn.commit().await.map_err(db_err)?;

        document_from(updated_doc).map_err(internal_err)
    }

    async fn get(
        &self,
        ctx: &WorkspaceCtx,
        id: DocumentId,
    ) -> Result<Option<Document>, DomainError> {
        document::Entity::find_by_id(id.0)
            .filter(document::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(document::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .map(document_from)
            .transpose()
            .map_err(internal_err)
    }

    async fn list_visible(
        &self,
        ctx: &WorkspaceCtx,
        principal: &Principal,
        after_id: Option<uuid::Uuid>,
        limit: u64,
    ) -> Result<Vec<DocumentSummary>, DomainError> {
        use sea_orm::FromQueryResult;

        #[derive(Debug, FromQueryResult)]
        struct Row {
            id: uuid::Uuid,
            workspace_id: uuid::Uuid,
            project_id: Option<uuid::Uuid>,
            folder_id: Option<uuid::Uuid>,
            title: String,
            slug: Option<String>,
            frontmatter: sea_orm::prelude::Json,
            current_revision_id: Option<uuid::Uuid>,
            current_revision_seq: i64,
            created_by_user_id: Option<uuid::Uuid>,
            created_by_api_key_id: Option<uuid::Uuid>,
            created_at: chrono::DateTime<chrono::Utc>,
            updated_at: chrono::DateTime<chrono::Utc>,
        }

        let mut values: Vec<sea_orm::Value> = Vec::new();
        values.push(ctx.workspace_id.0.into()); // $1

        let membership_clause;
        let principal_col;

        match principal {
            Principal::User(uid) => {
                principal_col = "user_id";
                values.push(uid.0.into()); // $2
                membership_clause = "EXISTS (
                        SELECT 1 FROM workspace_memberships
                        WHERE workspace_id = $1
                          AND user_id = $2
                    )"
                .to_string();
            }
            Principal::ApiKey(kid) => {
                principal_col = "api_key_id";
                values.push(kid.0.into()); // $2
                membership_clause = "FALSE".to_string();
            }
        }

        let cursor_cond = if let Some(cursor) = after_id {
            values.push(cursor.into());
            format!("AND d.id > ${}", values.len())
        } else {
            String::new()
        };

        let sql = format!(
            r#"
            SELECT d.id, d.workspace_id, d.project_id, d.folder_id, d.title, d.slug,
                   d.frontmatter, d.current_revision_id, d.current_revision_seq,
                   d.created_by_user_id, d.created_by_api_key_id, d.created_at, d.updated_at
            FROM documents d
            WHERE d.workspace_id = $1
              AND d.deleted_at IS NULL
              AND (
                    {membership_clause}
                    OR EXISTS (
                        SELECT 1 FROM permission_grants
                        WHERE workspace_id = $1
                          AND {principal_col} = $2
                          AND project_id IS NULL
                          AND folder_id IS NULL
                          AND document_id IS NULL
                          AND board_id IS NULL
                    )
                    OR EXISTS (
                        SELECT 1 FROM permission_grants
                        WHERE workspace_id = $1
                          AND {principal_col} = $2
                          AND document_id = d.id
                    )
              )
              {cursor_cond}
            ORDER BY d.id
            LIMIT {limit}
            "#,
        );

        let rows = Row::find_by_statement(sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            sql,
            values,
        ))
        .all(&self.conn)
        .await
        .map_err(db_err)?;

        rows.into_iter()
            .map(|r| {
                let current_revision_id = r
                    .current_revision_id
                    .ok_or_else(|| "document missing current_revision_id".to_string())?;

                Ok(DocumentSummary {
                    id: atlas_domain::ids::DocumentId(r.id),
                    workspace_id: atlas_domain::ids::WorkspaceId(r.workspace_id),
                    project_id: r.project_id.map(atlas_domain::ids::ProjectId),
                    folder_id: r.folder_id.map(atlas_domain::ids::FolderId),
                    title: r.title,
                    slug: r.slug,
                    frontmatter: r.frontmatter,
                    current_revision_id: atlas_domain::ids::RevisionId(current_revision_id),
                    current_revision_seq: r.current_revision_seq,
                    created_by_user_id: r.created_by_user_id.map(atlas_domain::ids::UserId),
                    created_by_api_key_id: r.created_by_api_key_id.map(atlas_domain::ids::ApiKeyId),
                    created_at: r.created_at,
                    updated_at: r.updated_at,
                })
            })
            .collect::<Result<Vec<_>, String>>()
            .map_err(internal_err)
    }

    async fn find_by_slug(
        &self,
        ctx: &WorkspaceCtx,
        slug: &str,
    ) -> Result<Option<Document>, DomainError> {
        document::Entity::find()
            .filter(document::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(document::Column::Slug.eq(slug))
            .filter(document::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .map(document_from)
            .transpose()
            .map_err(internal_err)
    }

    async fn rename(
        &self,
        ctx: &WorkspaceCtx,
        id: DocumentId,
        new_title: String,
    ) -> Result<Document, DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;

        let row = document::Entity::find_by_id(id.0)
            .filter(document::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(document::Column::DeletedAt.is_null())
            .lock_exclusive()
            .one(&txn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "document",
                id: id.0,
            })?;

        let mut active = row.into_active_model();
        active.title = Set(new_title.clone());
        active.updated_at = Set(Utc::now());
        let updated = active.update(&txn).await.map_err(db_err)?;

        // Sweep document_links: update target_title for any link targeting this doc.
        update_backlink_titles(&txn, ctx.workspace_id.0, id.0, &new_title)
            .await
            .map_err(db_err)?;

        txn.commit().await.map_err(db_err)?;

        document_from(updated).map_err(internal_err)
    }

    async fn update_content(
        &self,
        ctx: &WorkspaceCtx,
        id: DocumentId,
        expected_revision: RevisionId,
        new_content: &str,
    ) -> Result<Document, DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;

        let doc = document::Entity::find_by_id(id.0)
            .filter(document::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(document::Column::DeletedAt.is_null())
            .lock_exclusive()
            .one(&txn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "document",
                id: id.0,
            })?;

        let current_rev_uuid = doc.current_revision_id.ok_or(DomainError::NotFound {
            entity: "document.current_revision_id",
            id: id.0,
        })?;

        if current_rev_uuid != expected_revision.0 {
            let base_seq = find_revision_seq(&txn, id.0, expected_revision.0)
                .await
                .map_err(db_err)?;

            let Some(base_seq) = base_seq else {
                txn.rollback().await.map_err(db_err)?;
                return Err(DomainError::InvalidInput {
                    message: "base_revision_id is not a revision of this document".to_string(),
                });
            };

            let base_content = reconstruct_content_at(&txn, id.0, base_seq)
                .await
                .map_err(internal_err)?;

            let patch = create_revision_patch(&base_content, &doc.content);

            txn.rollback().await.map_err(db_err)?;

            return Err(DomainError::Conflict(RevisionConflict {
                document_id: id,
                current_revision_id: RevisionId(current_rev_uuid),
                current_seq: doc.current_revision_seq,
                base_to_current_patch: patch,
            }));
        }

        let patch = create_revision_patch(&doc.content, new_content);
        let next_seq = doc.current_revision_seq + 1;
        let is_anchor = is_anchor_seq(next_seq, self.anchor_interval);
        let rev_id = RevisionId::new();
        let (by_user, by_key) = actor_fields(&ctx.actor);
        let now = Utc::now();

        let rev_model = document_revision::ActiveModel {
            id: Set(rev_id.0),
            workspace_id: Set(ctx.workspace_id.0),
            document_id: Set(id.0),
            seq: Set(next_seq),
            patch: Set(Some(patch)),
            snapshot: Set(if is_anchor {
                Some(new_content.to_string())
            } else {
                None
            }),
            is_anchor: Set(is_anchor),
            created_by_user_id: Set(by_user),
            created_by_api_key_id: Set(by_key),
            created_at: Set(now),
        };
        rev_model.insert(&txn).await.map_err(db_err)?;

        let mut doc_active = doc.into_active_model();
        doc_active.content = Set(new_content.to_string());
        doc_active.current_revision_id = Set(Some(rev_id.0));
        doc_active.current_revision_seq = Set(next_seq);
        doc_active.updated_at = Set(now);
        let updated = doc_active.update(&txn).await.map_err(db_err)?;

        txn.commit().await.map_err(db_err)?;

        document_from(updated).map_err(internal_err)
    }

    async fn update_frontmatter(
        &self,
        ctx: &WorkspaceCtx,
        id: DocumentId,
        fm: serde_json::Value,
    ) -> Result<Document, DomainError> {
        let row = document::Entity::find_by_id(id.0)
            .filter(document::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(document::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "document",
                id: id.0,
            })?;

        let mut active = row.into_active_model();
        active.frontmatter = Set(fm);
        active.updated_at = Set(Utc::now());
        let updated = active.update(&self.conn).await.map_err(db_err)?;

        document_from(updated).map_err(internal_err)
    }

    async fn move_to(
        &self,
        ctx: &WorkspaceCtx,
        id: DocumentId,
        folder: Option<FolderId>,
        project: Option<ProjectId>,
    ) -> Result<(), DomainError> {
        let row = document::Entity::find_by_id(id.0)
            .filter(document::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(document::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "document",
                id: id.0,
            })?;

        let mut active = row.into_active_model();
        active.folder_id = Set(folder.map(|id| id.0));
        active.project_id = Set(project.map(|id| id.0));
        active.updated_at = Set(Utc::now());
        active.update(&self.conn).await.map_err(db_err)?;
        Ok(())
    }

    async fn soft_delete(&self, ctx: &WorkspaceCtx, id: DocumentId) -> Result<(), DomainError> {
        let row = document::Entity::find_by_id(id.0)
            .filter(document::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(document::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "document",
                id: id.0,
            })?;

        let mut active = row.into_active_model();
        active.deleted_at = Set(Some(Utc::now()));
        active.updated_at = Set(Utc::now());
        active.update(&self.conn).await.map_err(db_err)?;
        Ok(())
    }

    async fn history(
        &self,
        ctx: &WorkspaceCtx,
        id: DocumentId,
    ) -> Result<Vec<RevisionMeta>, DomainError> {
        let _ = document::Entity::find_by_id(id.0)
            .filter(document::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(document::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "document",
                id: id.0,
            })?;

        let rows = document_revision::Entity::find()
            .filter(document_revision::Column::DocumentId.eq(id.0))
            .order_by_asc(document_revision::Column::Seq)
            .all(&self.conn)
            .await
            .map_err(db_err)?;

        Ok(rows.into_iter().map(revision_meta_from).collect())
    }

    async fn content_at(
        &self,
        ctx: &WorkspaceCtx,
        id: DocumentId,
        seq: i64,
    ) -> Result<String, DomainError> {
        let _ = document::Entity::find_by_id(id.0)
            .filter(document::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(document::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "document",
                id: id.0,
            })?;

        reconstruct_content_at(&self.conn, id.0, seq)
            .await
            .map_err(internal_err)
    }
}

async fn find_revision_seq(
    conn: &impl sea_orm::ConnectionTrait,
    doc_id: Uuid,
    rev_id: Uuid,
) -> Result<Option<i64>, sea_orm::DbErr> {
    let row = document_revision::Entity::find_by_id(rev_id)
        .filter(document_revision::Column::DocumentId.eq(doc_id))
        .one(conn)
        .await?;

    Ok(row.map(|r| r.seq))
}

async fn reconstruct_content_at(
    conn: &impl sea_orm::ConnectionTrait,
    doc_id: Uuid,
    target_seq: i64,
) -> Result<String, String> {
    let anchor = document_revision::Entity::find()
        .filter(document_revision::Column::DocumentId.eq(doc_id))
        .filter(document_revision::Column::Seq.lte(target_seq))
        .filter(document_revision::Column::IsAnchor.eq(true))
        .order_by_desc(document_revision::Column::Seq)
        .one(conn)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| {
            format!("no anchor found for document {doc_id} at or before seq {target_seq}")
        })?;

    let anchor_snapshot = anchor
        .snapshot
        .ok_or_else(|| format!("anchor at seq {} has no snapshot", anchor.seq))?;

    if anchor.seq == target_seq {
        return Ok(anchor_snapshot);
    }

    let patches = document_revision::Entity::find()
        .filter(document_revision::Column::DocumentId.eq(doc_id))
        .filter(document_revision::Column::Seq.gt(anchor.seq))
        .filter(document_revision::Column::Seq.lte(target_seq))
        .order_by_asc(document_revision::Column::Seq)
        .all(conn)
        .await
        .map_err(|e| e.to_string())?;

    let patch_strings: Vec<&str> = patches
        .iter()
        .map(|r| {
            r.patch
                .as_deref()
                .ok_or_else(|| format!("revision at seq {} is missing patch", r.seq))
        })
        .collect::<Result<Vec<_>, _>>()?;

    reconstruct(&anchor_snapshot, &patch_strings).map_err(|e| e.to_string())
}

pub struct PgDocumentLinkRepo {
    pub conn: DatabaseConnection,
}

#[async_trait]
impl DocumentLinkRepo for PgDocumentLinkRepo {
    async fn replace_for_source(
        &self,
        ctx: &WorkspaceCtx,
        source: DocumentId,
        links: Vec<ExtractedLink>,
    ) -> Result<(), DomainError> {
        document_link::Entity::delete_many()
            .filter(document_link::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(document_link::Column::SourceDocumentId.eq(source.0))
            .exec(&self.conn)
            .await
            .map_err(db_err)?;

        for link in links {
            let model = document_link::ActiveModel {
                id: Set(Uuid::now_v7()),
                workspace_id: Set(ctx.workspace_id.0),
                source_document_id: Set(source.0),
                target_document_id: Set(link.target_document_id.map(|id| id.0)),
                target_title: Set(link.target_title),
                created_at: Set(Utc::now()),
            };
            model.insert(&self.conn).await.map_err(db_err)?;
        }

        Ok(())
    }

    async fn backlinks(
        &self,
        ctx: &WorkspaceCtx,
        target: DocumentId,
    ) -> Result<Vec<DocumentLink>, DomainError> {
        document_link::Entity::find()
            .filter(document_link::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(document_link::Column::TargetDocumentId.eq(target.0))
            .all(&self.conn)
            .await
            .map(|rows| rows.into_iter().map(document_link_from).collect())
            .map_err(db_err)
    }
}

pub struct PgAttachmentRepo {
    pub conn: DatabaseConnection,
}

#[async_trait]
impl AttachmentRepo for PgAttachmentRepo {
    async fn record(
        &self,
        ctx: &WorkspaceCtx,
        new: NewAttachment,
    ) -> Result<Attachment, DomainError> {
        let (by_user, by_key) = actor_fields(&ctx.actor);
        let model = attachment::ActiveModel {
            id: Set(AttachmentId::new().0),
            workspace_id: Set(ctx.workspace_id.0),
            document_id: Set(new.document_id.map(|id| id.0)),
            task_id: Set(new.task_id.map(|id| id.0)),
            file_name: Set(new.file_name),
            content_type: Set(new.content_type),
            size_bytes: Set(new.size_bytes),
            sha256: Set(new.sha256),
            created_by_user_id: Set(by_user),
            created_by_api_key_id: Set(by_key),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            deleted_at: Set(None),
        };
        model
            .insert(&self.conn)
            .await
            .map(attachment_from)
            .map_err(db_err)
    }

    async fn find(
        &self,
        ctx: &WorkspaceCtx,
        id: AttachmentId,
    ) -> Result<Option<Attachment>, DomainError> {
        attachment::Entity::find_by_id(id.0)
            .filter(attachment::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(attachment::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map(|opt| opt.map(attachment_from))
            .map_err(db_err)
    }

    async fn list_for_owner(
        &self,
        ctx: &WorkspaceCtx,
        owner: AttachmentOwner,
    ) -> Result<Vec<Attachment>, DomainError> {
        let q = attachment::Entity::find()
            .filter(attachment::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(attachment::Column::DeletedAt.is_null());

        let rows = match owner {
            AttachmentOwner::Document(doc_id) => q
                .filter(attachment::Column::DocumentId.eq(doc_id.0))
                .all(&self.conn)
                .await
                .map_err(db_err)?,
            AttachmentOwner::Task(task_id) => q
                .filter(attachment::Column::TaskId.eq(task_id.0))
                .all(&self.conn)
                .await
                .map_err(db_err)?,
        };

        Ok(rows.into_iter().map(attachment_from).collect())
    }

    async fn soft_delete(&self, ctx: &WorkspaceCtx, id: AttachmentId) -> Result<(), DomainError> {
        let row = attachment::Entity::find_by_id(id.0)
            .filter(attachment::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(attachment::Column::DeletedAt.is_null())
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "attachment",
                id: id.0,
            })?;

        let mut active = row.into_active_model();
        active.deleted_at = Set(Some(Utc::now()));
        active.updated_at = Set(Utc::now());
        active.update(&self.conn).await.map_err(db_err)?;
        Ok(())
    }
}

async fn update_backlink_titles(
    conn: &impl sea_orm::ConnectionTrait,
    workspace_id: Uuid,
    target_doc_id: Uuid,
    new_title: &str,
) -> Result<(), sea_orm::DbErr> {
    conn.execute_raw(sea_orm::Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "UPDATE document_links SET target_title = $1 WHERE workspace_id = $2 AND target_document_id = $3",
        [new_title.into(), workspace_id.into(), target_doc_id.into()],
    ))
    .await?;

    Ok(())
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

fn internal_err(msg: String) -> DomainError {
    DomainError::Internal { message: msg }
}
