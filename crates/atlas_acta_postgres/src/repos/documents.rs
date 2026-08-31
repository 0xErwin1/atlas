//! Repository implementations for the `document` and `document_link` ports,
//! moved from `atlas_server::persistence::repos::documents` (S4 PR7).
//!
//! `list_visible_with_folder_presence` reads `custos.users`/
//! `custos.permission_grants` by raw SQL, and `move_to_in` reads the
//! `folder` entity (moved here in PR1). Both are reads/same-crate reads, not
//! a cross-domain write composition, so they move with the repo unchanged
//! per design D6 — the same discipline PR6 applied to
//! `WorkspaceRepo::list_for_api_key`.
//!
//! `PgAttachmentRepo`, `PgAttachmentWriteIntentRepo`, and
//! `PgAttachmentLifecycle` stay in `atlas_server`: their trait/inherent
//! methods compose a Custos security-audit append
//! (`append_resource_deleted_in`, `crate::persistence::repos::security_audit`)
//! with Acta context, the same "must not depend on atlas_custos" boundary
//! `security_audit.rs`'s own doc comment already documents for
//! `atlas_custos_postgres`. `actor_fields` is duplicated verbatim in both
//! locations (a tiny, pure `(Option<Uuid>, Option<Uuid>)` mapper with no
//! Custos/atlas_server dependency) rather than shared, since the two call
//! sites now live in different crates.

use async_trait::async_trait;
use atlas_acta::actor::Actor;
use atlas_acta::actor::WorkspaceCtx;
use atlas_acta::document_lines::DocumentLineEdit;
use atlas_acta::document_lines::apply_document_line_edit;
use atlas_acta::entities::documents::Document;
use atlas_acta::entities::documents::DocumentLink;
use atlas_acta::entities::documents::DocumentSummary;
use atlas_acta::entities::documents::ExtractedLink;
use atlas_acta::entities::documents::LinkSource;
use atlas_acta::entities::documents::NewDocument;
use atlas_acta::entities::documents::RevisionMeta;
use atlas_acta::entities::documents::TaskDescriptionLinks;
use atlas_acta::ids::AttachmentId;
use atlas_acta::ids::DocumentId;
use atlas_acta::ids::FolderId;
use atlas_acta::ids::ProjectId;
use atlas_acta::ids::RevisionId;
use atlas_acta::ids::TaskId;
use atlas_acta::ids::WorkspaceId;
use atlas_acta::revision::create_revision_patch;
use atlas_acta::revision::is_anchor_seq;
use atlas_acta::revision::reconstruct;
use atlas_acta::wikilink::WikilinkTarget;
use atlas_core::error::DomainError;
use atlas_core::error::RevisionConflict;
use atlas_core::principal::Principal;
use atlas_core::slug::slugify;
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, ConnectionTrait, DatabaseConnection,
    EntityTrait, IntoActiveModel, QueryFilter, QueryOrder, QuerySelect, Statement,
    TransactionTrait,
};
use serde_json::json;
use uuid::Uuid;

use crate::entities::boards_tasks::task;
use crate::entities::documents::comment_attachment_draft;
use crate::entities::documents::{
    attachment, document, document_from, document_link, document_link_from, document_revision,
    revision_meta_from,
};
use crate::live_ancestors::{
    folder_chain_is_live_sql, live_document_chain, live_folder_chain, live_project,
    live_task_chain, project_is_live_sql, task_chain_is_live_sql,
};
use atlas_postgres::db_err;

pub use atlas_acta::ports::documents::DocumentLinkRepo;
pub use atlas_acta::ports::documents::DocumentRepo;

fn actor_fields(actor: &Actor) -> (Option<Uuid>, Option<Uuid>) {
    match actor {
        Actor::User(uid) => (Some(uid.0), None),
        Actor::ApiKey(kid) => (None, Some(kid.0)),
    }
}

fn internal_err(msg: String) -> DomainError {
    DomainError::Internal { message: msg }
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
        target_task_id: row
            .try_get::<Option<Uuid>>("", "link_target_task_id")
            .map_err(db_err)?
            .map(TaskId),
        target_attachment_id: row
            .try_get::<Option<Uuid>>("", "link_target_attachment_id")
            .map_err(db_err)?
            .map(AttachmentId),
        target_title: row.try_get("", "link_target_title").map_err(db_err)?,
        created_at: row
            .try_get::<chrono::DateTime<Utc>>("", "link_created_at")
            .map_err(db_err)?,
    }))
}

async fn update_backlink_titles(
    conn: &impl sea_orm::ConnectionTrait,
    workspace_id: Uuid,
    target_doc_id: Uuid,
    new_title: &str,
) -> Result<(), sea_orm::DbErr> {
    conn.execute_raw(sea_orm::Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "UPDATE acta.document_links SET target_title = $1 WHERE workspace_id = $2 AND target_document_id = $3",
        [new_title.into(), workspace_id.into(), target_doc_id.into()],
    ))
    .await?;

    Ok(())
}

fn derive_frontmatter(content: &str) -> serde_json::Value {
    let (yaml, _body) = atlas_acta::frontmatter::strip_frontmatter(content);
    atlas_acta::frontmatter::parse_frontmatter_yaml(yaml.unwrap_or(""))
}

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

    /// Reads the leading `max_chars` characters of each requested document's raw
    /// content, keyed by document id. Documents outside `ctx`'s workspace, deleted
    /// documents, and ids without a row are simply absent from the result.
    ///
    /// Callers must have already authorized every id: this is a projection helper
    /// for previews, not an access-controlled listing.
    pub async fn content_heads(
        &self,
        ctx: &WorkspaceCtx,
        ids: &[DocumentId],
        max_chars: u32,
    ) -> Result<std::collections::HashMap<Uuid, String>, DomainError> {
        use sea_orm::FromQueryResult;

        #[derive(Debug, FromQueryResult)]
        struct Row {
            id: Uuid,
            head: String,
        }

        if ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }

        let mut values: Vec<sea_orm::Value> =
            vec![ctx.workspace_id.0.into(), (max_chars as i32).into()];
        let placeholders = ids
            .iter()
            .map(|id| {
                values.push(id.0.into());
                format!("${}", values.len())
            })
            .collect::<Vec<_>>()
            .join(", ");

        let sql = format!(
            r#"
            SELECT d.id, left(d.content, $2) AS head
            FROM acta.documents d
            WHERE d.workspace_id = $1
              AND d.deleted_at IS NULL
              AND d.id IN ({placeholders})
            "#
        );

        let rows = Row::find_by_statement(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            sql,
            values,
        ))
        .all(&self.conn)
        .await
        .map_err(db_err)?;

        Ok(rows.into_iter().map(|row| (row.id, row.head)).collect())
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
            .filter(live_project("documents.project_id"))
            .filter(live_folder_chain("documents.folder_id"))
            .one(&self.conn)
            .await
            .map_err(db_err)?
            .map(document_from)
            .transpose()
            .map_err(internal_err)
    }

    async fn list_visible_with_folder_presence(
        &self,
        ctx: &WorkspaceCtx,
        principal: &Principal,
        project_filter: Option<ProjectId>,
        folder_presence: atlas_acta::ports::documents::FolderPresence,
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
                        SELECT 1 FROM acta.workspace_memberships
                        WHERE workspace_id = $1
                          AND user_id = $2
                    )
                    OR EXISTS (
                        SELECT 1 FROM custos.users
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

        let folder_presence_cond = match folder_presence {
            atlas_acta::ports::documents::FolderPresence::Any => String::new(),
            atlas_acta::ports::documents::FolderPresence::Unfiled => {
                "AND d.folder_id IS NULL".to_string()
            }
            atlas_acta::ports::documents::FolderPresence::Filed => {
                "AND d.folder_id IS NOT NULL".to_string()
            }
        };

        let sql = format!(
            r#"
            SELECT d.id, d.workspace_id, d.project_id, d.folder_id, d.title, d.slug,
                   d.frontmatter, d.current_revision_id, d.current_revision_seq,
                   d.created_by_user_id, d.created_by_api_key_id, d.created_at, d.updated_at
            FROM acta.documents d
            WHERE d.workspace_id = $1
              AND d.deleted_at IS NULL
              AND {project_live}
              AND {folder_live}
              AND (
                    {membership_clause}
                    OR EXISTS (
                        SELECT 1 FROM custos.permission_grants
                        WHERE workspace_id = $1
                          AND {principal_col} = $2
                          AND resource_ref = 'acta::workspace::' || $1::text
                    )
                    OR EXISTS (
                        SELECT 1 FROM custos.permission_grants
                        WHERE workspace_id = $1
                          AND {principal_col} = $2
                          AND resource_ref = 'acta::document::' || d.id::text
                    )
                    OR EXISTS (
                        SELECT 1 FROM custos.permission_grants
                        WHERE workspace_id = $1
                          AND {principal_col} = $2
                          AND resource_ref = 'acta::project::' || d.project_id::text
                    )
                    OR EXISTS (
                        WITH RECURSIVE ancestors AS (
                            SELECT f.id, f.parent_folder_id, f.project_id,
                                   ARRAY[f.id] AS path, 1 AS depth
                            FROM acta.folders f
                            WHERE f.id = d.folder_id
                              AND f.workspace_id = $1
                            UNION ALL
                            SELECT pf.id, pf.parent_folder_id, pf.project_id,
                                   a.path || pf.id, a.depth + 1
                            FROM acta.folders pf
                            JOIN ancestors a ON pf.id = a.parent_folder_id
                            WHERE pf.workspace_id = $1
                              AND NOT pf.id = ANY(a.path)
                              AND a.depth < 32
                        ), ancestor_refs AS (
                            SELECT 'acta::folder::' || id::text AS resource_ref FROM ancestors
                            UNION ALL
                            SELECT 'acta::project::' || project_id::text FROM ancestors
                            WHERE project_id IS NOT NULL
                        )
                        SELECT 1 FROM custos.permission_grants pg
                        JOIN ancestor_refs ON ancestor_refs.resource_ref = pg.resource_ref
                        WHERE pg.workspace_id = $1
                          AND pg.{principal_col} = $2
                    )
               )
               {project_cond}
               {folder_presence_cond}
               {cursor_cond}
            ORDER BY d.id
            LIMIT {limit}
            "#,
            project_live = project_is_live_sql("d.project_id"),
            folder_live = folder_chain_is_live_sql("d.folder_id"),
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
                    id: atlas_acta::ids::DocumentId(r.id),
                    workspace_id: atlas_acta::ids::WorkspaceId(r.workspace_id),
                    project_id: r.project_id.map(atlas_acta::ids::ProjectId),
                    folder_id: r.folder_id.map(atlas_acta::ids::FolderId),
                    title: r.title,
                    slug: r.slug,
                    frontmatter: r.frontmatter,
                    current_revision_id: atlas_acta::ids::RevisionId(current_revision_id),
                    current_revision_seq: r.current_revision_seq,
                    created_by_user_id: r.created_by_user_id.map(atlas_core::principal::UserId),
                    created_by_api_key_id: r
                        .created_by_api_key_id
                        .map(atlas_core::principal::ApiKeyId),
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
            .filter(live_project("documents.project_id"))
            .filter(live_folder_chain("documents.folder_id"))
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
            .filter(live_project("documents.project_id"))
            .filter(live_folder_chain("documents.folder_id"))
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
            .filter(live_project("documents.project_id"))
            .filter(live_folder_chain("documents.folder_id"))
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
            .filter(live_project("documents.project_id"))
            .filter(live_folder_chain("documents.folder_id"))
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

    let retained_draft = comment_attachment_draft::Entity::find()
        .filter(comment_attachment_draft::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(comment_attachment_draft::Column::DocumentId.eq(id.0))
        .one(conn)
        .await
        .map_err(db_err)?;
    if retained_draft.is_some() {
        return Err(DomainError::CommentDraftConflict {
            reason: "document has retained comment draft state".into(),
        });
    }

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

    update_locked_content_in(
        conn,
        ctx,
        doc,
        id,
        expected_revision,
        new_content,
        anchor_interval,
    )
    .await
}

/// Applies a line edit after locking the document and appends its resulting revision.
pub async fn edit_content_in(
    conn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
    id: DocumentId,
    expected_revision: RevisionId,
    edit: DocumentLineEdit,
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

    if doc.current_revision_id != Some(expected_revision.0) {
        let content = doc.content.clone();
        return update_locked_content_in(
            conn,
            ctx,
            doc,
            id,
            expected_revision,
            &content,
            anchor_interval,
        )
        .await;
    }

    let new_content = apply_document_line_edit(&doc.content, edit).map_err(|error| {
        DomainError::InvalidInput {
            message: format!("invalid document line edit: {error:?}"),
        }
    })?;

    if new_content == doc.content {
        return document_from(doc).map_err(internal_err);
    }

    update_locked_content_in(
        conn,
        ctx,
        doc,
        id,
        expected_revision,
        &new_content,
        anchor_interval,
    )
    .await
}

async fn update_locked_content_in(
    conn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
    doc: document::Model,
    id: DocumentId,
    expected_revision: RevisionId,
    new_content: &str,
    anchor_interval: u32,
) -> Result<Document, DomainError> {
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
            resource_id: id.0,
            current_revision_id: current_rev_uuid,
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
    use crate::entities::workspace_core::folder as folder_entity;

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
        .lock_exclusive()
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

/// Builds the row for one extracted wikilink.
///
/// Shared by every `replace_for_*` path so a source kind cannot drift into
/// storing a different set of target columns than the others.
fn link_model(
    ctx: &WorkspaceCtx,
    source: LinkSource,
    link: ExtractedLink,
) -> document_link::ActiveModel {
    let (source_document_id, source_task_id) = match source {
        LinkSource::Document(id) => (Some(id.0), None),
        LinkSource::Task(id) => (None, Some(id.0)),
    };

    document_link::ActiveModel {
        id: Set(Uuid::now_v7()),
        workspace_id: Set(ctx.workspace_id.0),
        source_document_id: Set(source_document_id),
        source_task_id: Set(source_task_id),
        target_document_id: Set(link.target_document_id.map(|id| id.0)),
        target_task_id: Set(link.target_task_id.map(|id| id.0)),
        target_attachment_id: Set(link.target_attachment_id.map(|id| id.0)),
        target_title: Set(link.target_title),
        created_at: Set(Utc::now()),
    }
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
            link_model(ctx, LinkSource::Task(source), link)
                .insert(conn)
                .await
                .map_err(db_err)?;
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

    /// Resolves a live task id by readable id inside an existing transaction.
    async fn find_task_id_by_readable_id_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        readable_id: &str,
    ) -> Result<Option<TaskId>, DomainError> {
        let row = task::Entity::find()
            .filter(task::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task::Column::ReadableId.eq(readable_id))
            .filter(task::Column::DeletedAt.is_null())
            .one(conn)
            .await
            .map_err(db_err)?;

        Ok(row.map(|t| TaskId(t.id)))
    }

    /// Resolves a live attachment id by file name among the attachments of the
    /// resource that contains the link.
    ///
    /// File names are not unique, so the most recently created match wins; the
    /// picker disambiguates at insertion time rather than leaving the reader to
    /// guess.
    async fn find_attachment_id_by_name_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        owner: LinkSource,
        file_name: &str,
    ) -> Result<Option<AttachmentId>, DomainError> {
        let scoped = match owner {
            LinkSource::Document(id) => attachment::Column::DocumentId.eq(id.0),
            LinkSource::Task(id) => attachment::Column::TaskId.eq(id.0),
        };

        let row = attachment::Entity::find()
            .filter(attachment::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(scoped)
            .filter(attachment::Column::FileName.eq(file_name))
            .filter(attachment::Column::DeletedAt.is_null())
            .order_by_desc(attachment::Column::CreatedAt)
            .order_by_desc(attachment::Column::Id)
            .one(conn)
            .await
            .map_err(db_err)?;

        Ok(row.map(|a| AttachmentId(a.id)))
    }

    /// Parses the wikilinks in `content` and resolves each one to its target.
    ///
    /// `source` is the resource the content belongs to: it becomes the link's
    /// source row and scopes `file:` addresses to that resource's attachments.
    /// A link whose target does not exist is kept with every target id `None`
    /// (a pending link) rather than dropped, so it starts resolving the moment
    /// the target appears.
    pub async fn extract_links_in(
        conn: &impl ConnectionTrait,
        ctx: &WorkspaceCtx,
        source: LinkSource,
        content: &str,
    ) -> Result<Vec<ExtractedLink>, DomainError> {
        let raw_links = atlas_acta::wikilink::parse_wikilinks(content);
        let mut extracted = Vec::with_capacity(raw_links.len());

        for raw in raw_links {
            let parsed = atlas_acta::wikilink::classify_wikilink(&raw);

            let mut link = ExtractedLink {
                target_title: parsed.display,
                ..ExtractedLink::default()
            };

            match parsed.target {
                WikilinkTarget::Task { readable_id } => {
                    link.target_task_id =
                        Self::find_task_id_by_readable_id_in(conn, ctx, &readable_id).await?;
                }
                WikilinkTarget::Note { slug } => {
                    link.target_document_id =
                        Self::find_document_id_by_slug_in(conn, ctx, &slug).await?;
                }
                WikilinkTarget::File { file_name } => {
                    link.target_attachment_id =
                        Self::find_attachment_id_by_name_in(conn, ctx, source, &file_name).await?;
                }
                WikilinkTarget::Document { id } => {
                    link.target_document_id =
                        Self::find_document_id_by_id_in(conn, ctx, DocumentId(id)).await?;
                }
                WikilinkTarget::Title => {
                    link.target_document_id =
                        Self::find_document_id_by_slug_in(conn, ctx, &slugify(&link.target_title))
                            .await?;
                }
            }

            extracted.push(link);
        }

        Ok(extracted)
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
            link_model(ctx, LinkSource::Document(source), link)
                .insert(&self.conn)
                .await
                .map_err(db_err)?;
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
            link_model(ctx, LinkSource::Task(source), link)
                .insert(&self.conn)
                .await
                .map_err(db_err)?;
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
                format!("SELECT t.description, dl.id AS link_id, dl.workspace_id AS link_workspace_id, \
                  dl.source_document_id AS link_source_document_id, dl.source_task_id AS link_source_task_id, \
                  dl.target_document_id AS link_target_document_id, dl.target_task_id AS link_target_task_id, \
                  dl.target_attachment_id AS link_target_attachment_id, dl.target_title AS link_target_title, \
                  dl.created_at AS link_created_at \
                  FROM acta.tasks t \
                  LEFT JOIN acta.document_links dl ON dl.workspace_id = t.workspace_id AND dl.source_task_id = t.id \
                  WHERE t.id = $1 AND t.workspace_id = $2 AND t.deleted_at IS NULL \
                    AND ({}) \
                  ORDER BY dl.created_at ASC NULLS LAST, dl.id ASC NULLS LAST",
                task_chain_is_live_sql("t.id"),
                ),
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
            .filter(live_document_chain("document_links.source_document_id"))
            .filter(live_task_chain("document_links.source_task_id"))
            .filter(live_document_chain("document_links.target_document_id"))
            .all(&self.conn)
            .await
            .map(|rows| rows.into_iter().map(document_link_from).collect())
            .map_err(db_err)
    }

    async fn backlinks_for_task(
        &self,
        ctx: &WorkspaceCtx,
        target: TaskId,
    ) -> Result<Vec<DocumentLink>, DomainError> {
        document_link::Entity::find()
            .filter(document_link::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(document_link::Column::TargetTaskId.eq(target.0))
            .filter(live_document_chain("document_links.source_document_id"))
            .filter(live_task_chain("document_links.source_task_id"))
            .filter(live_task_chain("document_links.target_task_id"))
            .order_by_asc(document_link::Column::CreatedAt)
            .order_by_asc(document_link::Column::Id)
            .all(&self.conn)
            .await
            .map(|rows| rows.into_iter().map(document_link_from).collect())
            .map_err(db_err)
    }
}
