use async_trait::async_trait;
use atlas_domain::{
    Actor, AttachmentStore, DomainError, RevisionConflict, WorkspaceCtx,
    entities::documents::{
        Attachment, AttachmentOwner, AttachmentWriteIntent, Document, DocumentLink,
        DocumentSummary, ExtractedLink, NewAttachment, NewDocument, RevisionMeta,
        TaskDescriptionLinks,
    },
    ids::{AttachmentId, DocumentId, FolderId, ProjectId, RevisionId, TaskId, WorkspaceId},
    permissions::Principal,
    revision::{create_revision_patch, is_anchor_seq, reconstruct},
};
use chrono::{DateTime, Utc};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, Statement,
    TransactionTrait,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{Postgres, pool::PoolConnection};
use uuid::Uuid;

use crate::persistence::entities::documents::{
    attachment, attachment_from, attachment_write_intent, attachment_write_intent_from, document,
    document_from, document_link, document_link_from, document_revision, revision_meta_from,
};

pub use atlas_domain::ports::documents::{
    AttachmentRepo, AttachmentWriteIntentRepo, DocumentLinkRepo, DocumentRepo,
};

pub struct PgDocumentRepo {
    pub conn: DatabaseConnection,
    pub anchor_interval: u32,
}

const ATTACHMENT_STORE_IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

struct DigestSessionLock {
    connection: PoolConnection<Postgres>,
    digest: String,
}

impl DigestSessionLock {
    async fn acquire(conn: &DatabaseConnection, digest: &str) -> Result<Self, DomainError> {
        let mut connection = conn
            .get_postgres_connection_pool()
            .acquire()
            .await
            .map_err(sqlx_err)?;

        sqlx::query("SELECT pg_advisory_lock(hashtextextended($1, 0))")
            .bind(digest)
            .execute(&mut *connection)
            .await
            .map_err(sqlx_err)?;

        Ok(Self {
            connection,
            digest: digest.into(),
        })
    }

    async fn release(mut self) -> Result<(), DomainError> {
        let unlocked: bool =
            sqlx::query_scalar("SELECT pg_advisory_unlock(hashtextextended($1, 0))")
                .bind(&self.digest)
                .fetch_one(&mut *self.connection)
                .await
                .map_err(sqlx_err)?;

        if unlocked {
            Ok(())
        } else {
            Err(DomainError::Internal {
                message: "attachment digest session lock was not held".into(),
            })
        }
    }
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
        let doc = create_in(&txn, ctx, new).await?;
        txn.commit().await.map_err(db_err)?;
        Ok(doc)
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
        project_filter: Option<ProjectId>,
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
                membership_clause = "(
                    EXISTS (
                        SELECT 1 FROM workspace_memberships
                        WHERE workspace_id = $1
                          AND user_id = $2
                    )
                    OR EXISTS (
                        SELECT 1 FROM users
                        WHERE id = $2
                          AND (is_root OR is_system_admin)
                          AND disabled_at IS NULL
                    )
                )"
                .to_string();
            }
            Principal::ApiKey(kid) => {
                principal_col = "api_key_id";
                values.push(kid.0.into()); // $2
                membership_clause = "FALSE".to_string();
            }
            Principal::Group(_) => {
                principal_col = "user_id";
                values.push(uuid::Uuid::nil().into());
                membership_clause = "FALSE".to_string();
            }
        }

        let cursor_cond = if let Some(cursor) = after_id {
            values.push(cursor.into());
            format!("AND d.id > ${}", values.len())
        } else {
            String::new()
        };

        let project_cond = if let Some(project_id) = project_filter {
            values.push(project_id.0.into());
            format!("AND d.project_id = ${}", values.len())
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
                    OR EXISTS (
                        SELECT 1 FROM permission_grants
                        WHERE workspace_id = $1
                          AND {principal_col} = $2
                          AND project_id IS NOT NULL
                          AND project_id = d.project_id
                    )
                    OR EXISTS (
                        WITH RECURSIVE ancestors AS (
                            SELECT f.id, f.parent_folder_id, f.project_id,
                                   ARRAY[f.id] AS path, 1 AS depth
                            FROM folders f
                            WHERE f.id = d.folder_id
                              AND f.workspace_id = $1
                            UNION ALL
                            SELECT pf.id, pf.parent_folder_id, pf.project_id,
                                   a.path || pf.id, a.depth + 1
                            FROM folders pf
                            JOIN ancestors a ON pf.id = a.parent_folder_id
                            WHERE pf.workspace_id = $1
                              AND NOT pf.id = ANY(a.path)
                              AND a.depth < 32
                        )
                        SELECT 1 FROM permission_grants pg
                        WHERE pg.workspace_id = $1
                          AND pg.{principal_col} = $2
                          AND (
                                pg.folder_id IN (SELECT id FROM ancestors)
                                OR (
                                    pg.project_id IS NOT NULL
                                    AND pg.project_id IN (
                                        SELECT project_id FROM ancestors
                                        WHERE project_id IS NOT NULL
                                    )
                                )
                          )
                    )
              )
              {project_cond}
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

    async fn list_in_folder(
        &self,
        ctx: &WorkspaceCtx,
        folder: FolderId,
    ) -> Result<Vec<Document>, DomainError> {
        document::Entity::find()
            .filter(document::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(document::Column::FolderId.eq(folder.0))
            .filter(document::Column::DeletedAt.is_null())
            .all(&self.conn)
            .await
            .map_err(db_err)?
            .into_iter()
            .map(document_from)
            .collect::<Result<Vec<_>, String>>()
            .map_err(internal_err)
    }

    async fn rename(
        &self,
        ctx: &WorkspaceCtx,
        id: DocumentId,
        new_title: String,
    ) -> Result<Document, DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;
        let doc = rename_in(&txn, ctx, id, new_title).await?;
        txn.commit().await.map_err(db_err)?;
        Ok(doc)
    }

    async fn update_content(
        &self,
        ctx: &WorkspaceCtx,
        id: DocumentId,
        expected_revision: RevisionId,
        new_content: &str,
    ) -> Result<Document, DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;
        let doc = update_content_in(
            &txn,
            ctx,
            id,
            expected_revision,
            new_content,
            self.anchor_interval,
        )
        .await?;
        txn.commit().await.map_err(db_err)?;
        Ok(doc)
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
        move_to_in(&self.conn, ctx, id, folder, project).await
    }

    async fn soft_delete(&self, ctx: &WorkspaceCtx, id: DocumentId) -> Result<(), DomainError> {
        soft_delete_in(&self.conn, ctx, id).await
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
            .filter(document_revision::Column::WorkspaceId.eq(ctx.workspace_id.0))
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

        reconstruct_content_at(&self.conn, ctx.workspace_id.0, id.0, seq)
            .await
            .map_err(internal_err)
    }
}

// ─── Extracted mutation primitives ───────────────────────────────────────────
//
// Each `*_in` function performs exactly one logical mutation on `conn` (which
// may be a DatabaseTransaction or a DatabaseConnection). The caller is
// responsible for wrapping in a transaction and committing or rolling back.

/// Inserts a new document and its first revision within the given connection.
///
/// Used by both `PgDocumentRepo::create` (which provides its own txn) and
/// `DocumentService::create` (which also emits an outbox event in the same txn).
pub async fn create_in(
    conn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
    new: NewDocument,
) -> Result<Document, DomainError> {
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
    let inserted_doc = doc_model.insert(conn).await.map_err(db_err)?;

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
    rev_model.insert(conn).await.map_err(db_err)?;

    let mut doc_active = inserted_doc.into_active_model();
    doc_active.current_revision_id = Set(Some(rev_id.0));
    doc_active.current_revision_seq = Set(1);
    let updated_doc = doc_active.update(conn).await.map_err(db_err)?;

    document_from(updated_doc).map_err(internal_err)
}

/// Updates a document's title and sweeps backlink titles within `conn`.
///
/// Used by both `PgDocumentRepo::rename` and `DocumentService::rename`.
pub async fn rename_in(
    conn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
    id: DocumentId,
    new_title: String,
) -> Result<Document, DomainError> {
    let row = document::Entity::find_by_id(id.0)
        .filter(document::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(document::Column::DeletedAt.is_null())
        .lock_exclusive()
        .one(conn)
        .await
        .map_err(db_err)?
        .ok_or(DomainError::NotFound {
            entity: "document",
            id: id.0,
        })?;

    let mut active = row.into_active_model();
    active.title = Set(new_title.clone());
    active.updated_at = Set(Utc::now());
    let updated = active.update(conn).await.map_err(db_err)?;

    update_backlink_titles(conn, ctx.workspace_id.0, id.0, &new_title)
        .await
        .map_err(db_err)?;

    document_from(updated).map_err(internal_err)
}

/// Appends a content revision for a document within `conn`.
///
/// Returns `DomainError::Conflict` when `expected_revision` is not the current
/// head (CAS semantics). The caller is responsible for rolling back on error.
pub async fn update_content_in(
    conn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
    id: DocumentId,
    expected_revision: RevisionId,
    new_content: &str,
    anchor_interval: u32,
) -> Result<Document, DomainError> {
    let doc = document::Entity::find_by_id(id.0)
        .filter(document::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(document::Column::DeletedAt.is_null())
        .lock_exclusive()
        .one(conn)
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
        let base_seq = find_revision_seq(conn, ctx.workspace_id.0, id.0, expected_revision.0)
            .await
            .map_err(db_err)?;

        let Some(base_seq) = base_seq else {
            return Err(DomainError::InvalidInput {
                message: "base_revision_id is not a revision of this document".to_string(),
            });
        };

        let base_content = reconstruct_content_at(conn, ctx.workspace_id.0, id.0, base_seq)
            .await
            .map_err(internal_err)?;

        let patch = create_revision_patch(&base_content, &doc.content);

        return Err(DomainError::Conflict(RevisionConflict {
            document_id: id,
            current_revision_id: RevisionId(current_rev_uuid),
            current_seq: doc.current_revision_seq,
            base_to_current_patch: patch,
        }));
    }

    let patch = create_revision_patch(&doc.content, new_content);
    let next_seq = doc.current_revision_seq + 1;
    let is_anchor = is_anchor_seq(next_seq, anchor_interval);
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
    rev_model.insert(conn).await.map_err(db_err)?;

    let frontmatter = derive_frontmatter(new_content);

    let mut doc_active = doc.into_active_model();
    doc_active.content = Set(new_content.to_string());
    doc_active.frontmatter = Set(frontmatter);
    doc_active.current_revision_id = Set(Some(rev_id.0));
    doc_active.current_revision_seq = Set(next_seq);
    doc_active.updated_at = Set(now);
    let updated = doc_active.update(conn).await.map_err(db_err)?;

    document_from(updated).map_err(internal_err)
}

/// Moves a document to a different folder and/or project within `conn`.
///
/// When `folder` is `Some`, the target folder dictates the project so the two
/// fields cannot desync. When `folder` is `None`, `project` is used directly.
pub async fn move_to_in(
    conn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
    id: DocumentId,
    folder: Option<FolderId>,
    project: Option<ProjectId>,
) -> Result<(), DomainError> {
    use crate::persistence::entities::workspace_core::folder as folder_entity;

    let row = document::Entity::find_by_id(id.0)
        .filter(document::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(document::Column::DeletedAt.is_null())
        .one(conn)
        .await
        .map_err(db_err)?
        .ok_or(DomainError::NotFound {
            entity: "document",
            id: id.0,
        })?;

    let (target_folder_id, target_project_id) = match folder {
        Some(folder_id) => {
            let folder_row = folder_entity::Entity::find_by_id(folder_id.0)
                .filter(folder_entity::Column::WorkspaceId.eq(ctx.workspace_id.0))
                .filter(folder_entity::Column::DeletedAt.is_null())
                .one(conn)
                .await
                .map_err(db_err)?
                .ok_or(DomainError::InvalidInput {
                    message: "target folder does not exist in this workspace".to_string(),
                })?;

            (Some(folder_id.0), folder_row.project_id)
        }
        None => (None, project.map(|id| id.0)),
    };

    let mut active = row.into_active_model();
    active.folder_id = Set(target_folder_id);
    active.project_id = Set(target_project_id);
    active.updated_at = Set(Utc::now());
    active.update(conn).await.map_err(db_err)?;
    Ok(())
}

/// Soft-deletes a document by setting `deleted_at` within `conn`.
pub async fn soft_delete_in(
    conn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
    id: DocumentId,
) -> Result<(), DomainError> {
    let row = document::Entity::find_by_id(id.0)
        .filter(document::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(document::Column::DeletedAt.is_null())
        .one(conn)
        .await
        .map_err(db_err)?
        .ok_or(DomainError::NotFound {
            entity: "document",
            id: id.0,
        })?;

    let mut active = row.into_active_model();
    active.deleted_at = Set(Some(Utc::now()));
    active.updated_at = Set(Utc::now());
    active.update(conn).await.map_err(db_err)?;
    Ok(())
}

async fn find_revision_seq(
    conn: &impl sea_orm::ConnectionTrait,
    workspace_id: Uuid,
    doc_id: Uuid,
    rev_id: Uuid,
) -> Result<Option<i64>, sea_orm::DbErr> {
    let row = document_revision::Entity::find_by_id(rev_id)
        .filter(document_revision::Column::WorkspaceId.eq(workspace_id))
        .filter(document_revision::Column::DocumentId.eq(doc_id))
        .one(conn)
        .await?;

    Ok(row.map(|r| r.seq))
}

async fn reconstruct_content_at(
    conn: &impl sea_orm::ConnectionTrait,
    workspace_id: Uuid,
    doc_id: Uuid,
    target_seq: i64,
) -> Result<String, String> {
    let anchor = document_revision::Entity::find()
        .filter(document_revision::Column::WorkspaceId.eq(workspace_id))
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
        .filter(document_revision::Column::WorkspaceId.eq(workspace_id))
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

impl PgDocumentLinkRepo {
    /// Replaces the link set for a task source inside an existing transaction.
    ///
    /// The delete and the N inserts run on `conn`, which may be the caller's
    /// `DatabaseTransaction`, so wikilink persistence joins the task write and
    /// activity append in a single atomic unit (no torn link state).
    /// Lists the current wikilink target titles for a task source, inside an
    /// existing transaction. Used to diff the previous link set against a new one
    /// so only newly-added wikilinks emit a `DocumentMentioned` activity, rather
    /// than re-emitting every link on each description edit.
    pub async fn list_titles_for_task_source_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        source: TaskId,
    ) -> Result<Vec<String>, DomainError> {
        let rows = document_link::Entity::find()
            .filter(document_link::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(document_link::Column::SourceTaskId.eq(source.0))
            .all(conn)
            .await
            .map_err(db_err)?;

        Ok(rows.into_iter().map(|r| r.target_title).collect())
    }

    pub async fn replace_for_task_source_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        source: TaskId,
        links: Vec<ExtractedLink>,
    ) -> Result<(), DomainError> {
        document_link::Entity::delete_many()
            .filter(document_link::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(document_link::Column::SourceTaskId.eq(source.0))
            .exec(conn)
            .await
            .map_err(db_err)?;

        for link in links {
            let model = document_link::ActiveModel {
                id: Set(Uuid::now_v7()),
                workspace_id: Set(ctx.workspace_id.0),
                source_document_id: Set(None),
                source_task_id: Set(Some(source.0)),
                target_document_id: Set(link.target_document_id.map(|id| id.0)),
                target_title: Set(link.target_title),
                created_at: Set(Utc::now()),
            };
            model.insert(conn).await.map_err(db_err)?;
        }

        Ok(())
    }

    /// Resolves a document id by slug inside an existing transaction.
    ///
    /// Returns `None` when no live document matches the slug; callers store such
    /// wikilinks as pending (target_document_id NULL), consistent with E04.
    pub async fn find_document_id_by_slug_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        slug: &str,
    ) -> Result<Option<DocumentId>, DomainError> {
        let row = document::Entity::find()
            .filter(document::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(document::Column::Slug.eq(slug))
            .filter(document::Column::DeletedAt.is_null())
            .one(conn)
            .await
            .map_err(db_err)?;

        Ok(row.map(|d| DocumentId(d.id)))
    }

    /// Verifies a document id refers to a live document in this workspace, inside
    /// an existing transaction.
    ///
    /// Returns `Some(id)` when a matching live document exists, `None` otherwise;
    /// callers store an unresolved id-bound wikilink as pending
    /// (target_document_id NULL), consistent with E04.
    pub async fn find_document_id_by_id_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        id: DocumentId,
    ) -> Result<Option<DocumentId>, DomainError> {
        let row = document::Entity::find_by_id(id.0)
            .filter(document::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(document::Column::DeletedAt.is_null())
            .one(conn)
            .await
            .map_err(db_err)?;

        Ok(row.map(|d| DocumentId(d.id)))
    }
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
                source_document_id: Set(Some(source.0)),
                source_task_id: Set(None),
                target_document_id: Set(link.target_document_id.map(|id| id.0)),
                target_title: Set(link.target_title),
                created_at: Set(Utc::now()),
            };
            model.insert(&self.conn).await.map_err(db_err)?;
        }

        Ok(())
    }

    async fn replace_for_task_source(
        &self,
        ctx: &WorkspaceCtx,
        source: TaskId,
        links: Vec<ExtractedLink>,
    ) -> Result<(), DomainError> {
        document_link::Entity::delete_many()
            .filter(document_link::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(document_link::Column::SourceTaskId.eq(source.0))
            .exec(&self.conn)
            .await
            .map_err(db_err)?;

        for link in links {
            let model = document_link::ActiveModel {
                id: Set(Uuid::now_v7()),
                workspace_id: Set(ctx.workspace_id.0),
                source_document_id: Set(None),
                source_task_id: Set(Some(source.0)),
                target_document_id: Set(link.target_document_id.map(|id| id.0)),
                target_title: Set(link.target_title),
                created_at: Set(Utc::now()),
            };
            model.insert(&self.conn).await.map_err(db_err)?;
        }

        Ok(())
    }

    async fn outgoing_for_task(
        &self,
        ctx: &WorkspaceCtx,
        source: TaskId,
    ) -> Result<Option<TaskDescriptionLinks>, DomainError> {
        let rows = self
            .conn
            .query_all_raw(Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Postgres,
                "SELECT t.description, dl.id AS link_id, dl.workspace_id AS link_workspace_id, \
                 dl.source_document_id AS link_source_document_id, dl.source_task_id AS link_source_task_id, \
                 dl.target_document_id AS link_target_document_id, dl.target_title AS link_target_title, \
                 dl.created_at AS link_created_at \
                 FROM tasks t \
                 LEFT JOIN document_links dl ON dl.workspace_id = t.workspace_id AND dl.source_task_id = t.id \
                 WHERE t.id = $1 AND t.workspace_id = $2 AND t.deleted_at IS NULL \
                 ORDER BY dl.created_at ASC NULLS LAST, dl.id ASC NULLS LAST",
                [source.0.into(), ctx.workspace_id.0.into()],
            ))
            .await
            .map_err(db_err)?;

        let Some(first) = rows.first() else {
            return Ok(None);
        };

        let description: String = first.try_get("", "description").map_err(db_err)?;
        let links = rows
            .into_iter()
            .filter_map(|row| document_link_from_snapshot_row(&row).transpose())
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Some(TaskDescriptionLinks { description, links }))
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
            comment_id: Set(new.comment_id.map(|id| id.0)),
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
            AttachmentOwner::Comment(comment_id) => q
                .filter(attachment::Column::CommentId.eq(comment_id.0))
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

pub struct PgAttachmentWriteIntentRepo {
    pub conn: DatabaseConnection,
}

pub struct PgAttachmentLifecycle;

impl PgAttachmentLifecycle {
    /// Finishes a committed comment-attachment purge while holding the same
    /// digest lock used by writes and stale-intent reconciliation.
    pub async fn finish_purge_digest(
        conn: &DatabaseConnection,
        store: &dyn AttachmentStore,
        digest: &str,
    ) -> Result<(), DomainError> {
        let lock = DigestSessionLock::acquire(conn, digest).await?;
        let result = async {
            let intent = attachment_write_intent::Entity::find()
                .filter(attachment_write_intent::Column::Digest.eq(digest))
                .one(conn)
                .await
                .map_err(db_err)?;

            let Some(intent) = intent else {
                return Ok(());
            };

            let has_live_reference = attachment::Entity::find()
                .filter(attachment::Column::Sha256.eq(digest))
                .filter(attachment::Column::DeletedAt.is_null())
                .one(conn)
                .await
                .map_err(db_err)?
                .is_some();

            if !has_live_reference {
                bounded_store_delete(store, digest).await?;
            }

            attachment_write_intent::Entity::delete_by_id(intent.id)
                .exec(conn)
                .await
                .map(|_| ())
                .map_err(db_err)
        }
        .await;
        let unlock = lock.release().await;

        result?;
        unlock
    }

    pub async fn run_reconciler(
        conn: DatabaseConnection,
        store: std::sync::Arc<dyn AttachmentStore>,
        shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        Self::run_reconciler_with_timing(
            conn,
            store,
            shutdown,
            std::time::Duration::from_secs(300),
            chrono::Duration::minutes(10),
        )
        .await;
    }

    pub async fn run_reconciler_with_timing(
        conn: DatabaseConnection,
        store: std::sync::Arc<dyn AttachmentStore>,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
        interval_period: std::time::Duration,
        stale_after: chrono::Duration,
    ) {
        let mut interval = tokio::time::interval(interval_period);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let older_than = Utc::now() - stale_after;
                    if let Err(error) = Self::reconcile_stale(&conn, store.as_ref(), older_than).await {
                        tracing::warn!(%error, "attachment intent reconciliation failed");
                    }
                }
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { return; }
                }
            }
        }
    }

    pub async fn store_and_record(
        conn: &DatabaseConnection,
        ctx: &WorkspaceCtx,
        new: NewAttachment,
        data: &[u8],
        store: &dyn AttachmentStore,
    ) -> Result<Attachment, DomainError> {
        let digest = hex_digest(data);
        PgAttachmentWriteIntentRepo { conn: conn.clone() }
            .create_if_absent(digest.clone())
            .await?;

        let lock = DigestSessionLock::acquire(conn, &digest).await?;
        let result = async {
            PgAttachmentWriteIntentRepo { conn: conn.clone() }
                .create_if_absent(digest.clone())
                .await?;

            let stored = bounded_store_put(store, data).await?;
            if stored != digest {
                return Err(DomainError::Internal {
                    message: "attachment store returned an unexpected digest".into(),
                });
            }

            let txn = conn.begin().await.map_err(db_err)?;
            let attachment = PgAttachmentRepo::record_in(&txn, ctx, new, digest).await?;
            attachment_write_intent::Entity::delete_many()
                .filter(attachment_write_intent::Column::Digest.eq(&attachment.sha256))
                .exec(&txn)
                .await
                .map_err(db_err)?;
            txn.commit().await.map_err(db_err)?;
            Ok(attachment)
        }
        .await;
        let unlock = lock.release().await;

        let attachment = result?;
        unlock?;

        Ok(attachment)
    }

    pub async fn reconcile_stale(
        conn: &DatabaseConnection,
        store: &dyn AttachmentStore,
        older_than: DateTime<Utc>,
    ) -> Result<(), DomainError> {
        let intents = attachment_write_intent::Entity::find()
            .filter(attachment_write_intent::Column::CreatedAt.lt(older_than))
            .order_by_asc(attachment_write_intent::Column::CreatedAt)
            .order_by_asc(attachment_write_intent::Column::Id)
            .all(conn)
            .await
            .map_err(db_err)?;

        for intent in intents {
            if let Err(error) = Self::reconcile_intent(conn, store, older_than, intent).await {
                tracing::warn!(%error, "attachment intent cleanup failed");
            }
        }

        Ok(())
    }

    async fn reconcile_intent(
        conn: &DatabaseConnection,
        store: &dyn AttachmentStore,
        older_than: DateTime<Utc>,
        intent: attachment_write_intent::Model,
    ) -> Result<(), DomainError> {
        let current = attachment_write_intent::Entity::find_by_id(intent.id)
            .filter(attachment_write_intent::Column::CreatedAt.lt(older_than))
            .one(conn)
            .await
            .map_err(db_err)?;

        let Some(current) = current else {
            return Ok(());
        };

        Self::finish_purge_digest(conn, store, &current.digest).await
    }
}

async fn bounded_store_put(
    store: &dyn AttachmentStore,
    data: &[u8],
) -> Result<String, DomainError> {
    tokio::time::timeout(ATTACHMENT_STORE_IO_TIMEOUT, store.put(data))
        .await
        .map_err(|_| attachment_store_timeout("put"))?
}

async fn bounded_store_delete(
    store: &dyn AttachmentStore,
    digest: &str,
) -> Result<(), DomainError> {
    tokio::time::timeout(ATTACHMENT_STORE_IO_TIMEOUT, store.delete(digest))
        .await
        .map_err(|_| attachment_store_timeout("delete"))?
}

fn attachment_store_timeout(operation: &str) -> DomainError {
    DomainError::Internal {
        message: format!("attachment store {operation} timed out"),
    }
}

fn sqlx_err(error: sqlx::Error) -> DomainError {
    DomainError::Internal {
        message: error.to_string(),
    }
}

#[async_trait]
impl AttachmentWriteIntentRepo for PgAttachmentWriteIntentRepo {
    async fn create(&self, digest: String) -> Result<AttachmentWriteIntent, DomainError> {
        attachment_write_intent::ActiveModel {
            id: Set(Uuid::now_v7()),
            digest: Set(digest),
            created_at: Set(Utc::now()),
        }
        .insert(&self.conn)
        .await
        .map(attachment_write_intent_from)
        .map_err(db_err)
    }

    async fn remove(&self, digest: &str) -> Result<(), DomainError> {
        attachment_write_intent::Entity::delete_many()
            .filter(attachment_write_intent::Column::Digest.eq(digest))
            .exec(&self.conn)
            .await
            .map(|_| ())
            .map_err(db_err)
    }

    async fn list_stale(
        &self,
        older_than: DateTime<Utc>,
    ) -> Result<Vec<AttachmentWriteIntent>, DomainError> {
        attachment_write_intent::Entity::find()
            .filter(attachment_write_intent::Column::CreatedAt.lt(older_than))
            .all(&self.conn)
            .await
            .map(|rows| rows.into_iter().map(attachment_write_intent_from).collect())
            .map_err(db_err)
    }
}

impl PgAttachmentWriteIntentRepo {
    async fn create_if_absent(&self, digest: String) -> Result<(), DomainError> {
        self.conn
            .execute_unprepared(&format!(
                "INSERT INTO attachment_write_intents (id, digest, created_at) VALUES ('{}', '{}', now()) ON CONFLICT (digest) DO NOTHING",
                Uuid::now_v7(), digest
            ))
            .await
            .map(|_| ())
            .map_err(db_err)
    }
}

impl PgAttachmentRepo {
    async fn record_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        new: NewAttachment,
        sha256: String,
    ) -> Result<Attachment, DomainError> {
        let (by_user, by_key) = actor_fields(&ctx.actor);
        attachment::ActiveModel {
            id: Set(AttachmentId::new().0),
            workspace_id: Set(ctx.workspace_id.0),
            document_id: Set(new.document_id.map(|id| id.0)),
            task_id: Set(new.task_id.map(|id| id.0)),
            comment_id: Set(new.comment_id.map(|id| id.0)),
            file_name: Set(new.file_name),
            content_type: Set(new.content_type),
            size_bytes: Set(new.size_bytes),
            sha256: Set(sha256),
            created_by_user_id: Set(by_user),
            created_by_api_key_id: Set(by_key),
            created_at: Set(Utc::now()),
            updated_at: Set(Utc::now()),
            deleted_at: Set(None),
        }
        .insert(conn)
        .await
        .map(attachment_from)
        .map_err(db_err)
    }
}

fn hex_digest(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
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

fn derive_frontmatter(content: &str) -> serde_json::Value {
    let (yaml, _body) = atlas_domain::frontmatter::strip_frontmatter(content);
    atlas_domain::frontmatter::parse_frontmatter_yaml(yaml.unwrap_or(""))
}

fn actor_fields(actor: &Actor) -> (Option<Uuid>, Option<Uuid>) {
    match actor {
        Actor::User(uid) => (Some(uid.0), None),
        Actor::ApiKey(kid) => (None, Some(kid.0)),
    }
}

fn document_link_from_snapshot_row(
    row: &sea_orm::QueryResult,
) -> Result<Option<DocumentLink>, DomainError> {
    let id: Option<Uuid> = row.try_get("", "link_id").map_err(db_err)?;
    let Some(id) = id else {
        return Ok(None);
    };

    Ok(Some(DocumentLink {
        id: DocumentId(id),
        workspace_id: WorkspaceId(row.try_get("", "link_workspace_id").map_err(db_err)?),
        source_document_id: row
            .try_get::<Option<Uuid>>("", "link_source_document_id")
            .map_err(db_err)?
            .map(DocumentId),
        source_task_id: row
            .try_get::<Option<Uuid>>("", "link_source_task_id")
            .map_err(db_err)?
            .map(TaskId),
        target_document_id: row
            .try_get::<Option<Uuid>>("", "link_target_document_id")
            .map_err(db_err)?
            .map(DocumentId),
        target_title: row.try_get("", "link_target_title").map_err(db_err)?,
        created_at: row
            .try_get::<DateTime<Utc>>("", "link_created_at")
            .map_err(db_err)?,
    }))
}

fn db_err(e: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: e.to_string(),
    }
}

fn internal_err(msg: String) -> DomainError {
    DomainError::Internal { message: msg }
}
