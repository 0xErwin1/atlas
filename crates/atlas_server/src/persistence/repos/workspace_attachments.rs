use async_trait::async_trait;
use atlas_acta::actor::WorkspaceCtx;
use atlas_acta::entities::documents::Attachment;
use atlas_acta::entities::documents::AttachmentOwnerKind;
use atlas_acta::entities::documents::AttachmentOwnerRef;
use atlas_acta::entities::documents::WorkspaceAttachment;
use atlas_acta::entities::documents::WorkspaceAttachmentQuery;
use atlas_acta::ids::AttachmentId;
use atlas_acta::ids::CommentId;
use atlas_acta::ids::DocumentId;
use atlas_acta::ids::TaskId;
use atlas_acta::ids::WorkspaceId as DomainWsId;
use atlas_acta::ports::documents::WorkspaceAttachmentRepo;
use atlas_core::error::DomainError;
use atlas_core::principal::ApiKeyId;
use atlas_core::principal::Principal;
use atlas_core::principal::UserId;
use chrono::{DateTime, Utc};
use sea_orm::{DatabaseConnection, FromQueryResult, Statement};

use crate::persistence::{
    live_ancestors::{board_chain_is_live_sql, folder_chain_is_live_sql, project_is_live_sql},
    repos::search::{build_doc_permission, build_task_permission},
};
use atlas_postgres::db_err;

pub struct PgWorkspaceAttachmentRepo {
    pub conn: DatabaseConnection,
}

impl PgWorkspaceAttachmentRepo {
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }
}

/// One attachment row joined to whichever resource owns it. Both arms of the
/// UNION expose this exact column list.
#[derive(Debug, FromQueryResult)]
struct WorkspaceAttachmentRow {
    id: uuid::Uuid,
    workspace_id: uuid::Uuid,
    document_id: Option<uuid::Uuid>,
    task_id: Option<uuid::Uuid>,
    comment_id: Option<uuid::Uuid>,
    file_name: String,
    content_type: String,
    size_bytes: i64,
    sha256: String,
    created_by_user_id: Option<uuid::Uuid>,
    created_by_api_key_id: Option<uuid::Uuid>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    owner_kind: String,
    owner_title: String,
    owner_document_id: Option<uuid::Uuid>,
    owner_document_slug: Option<String>,
    owner_task_id: Option<uuid::Uuid>,
    owner_task_readable_id: Option<String>,
    owner_project_slug: Option<String>,
}

/// The attachment columns every arm selects, plus the comment join both share.
///
/// A comment-owned attachment resolves through its comment to the comment's
/// parent, so the join is `LEFT` and a soft-deleted comment drops the row when
/// the arm's own owner join is inner.
const ATTACHMENT_COLUMNS: &str = "
    a.id,
    a.workspace_id,
    a.document_id,
    a.task_id,
    a.comment_id,
    a.file_name,
    a.content_type,
    a.size_bytes,
    a.sha256,
    a.created_by_user_id,
    a.created_by_api_key_id,
    a.created_at,
    a.updated_at";

const ATTACHMENT_SOURCE: &str = "
    FROM acta.attachments a
    LEFT JOIN comments c ON c.id = a.comment_id AND c.workspace_id = $1 AND c.deleted_at IS NULL";

/// Attachments still hanging off an unpublished comment draft are private to
/// their author until the comment is posted, so they never enter the listing.
const ATTACHMENT_BASE_WHERE: &str = "
    a.workspace_id = $1
    AND a.deleted_at IS NULL
    AND a.draft_id IS NULL";

#[async_trait]
impl WorkspaceAttachmentRepo for PgWorkspaceAttachmentRepo {
    async fn list(
        &self,
        ctx: &WorkspaceCtx,
        principal: &Principal,
        query: &WorkspaceAttachmentQuery,
        bypass: bool,
    ) -> Result<Vec<WorkspaceAttachment>, DomainError> {
        let mut values: Vec<sea_orm::Value> = vec![ctx.workspace_id.0.into()];
        let clauses = PrincipalClauses::build(principal, bypass, &mut values);

        let filters = build_filters(query, &mut values);

        let document_perm =
            build_doc_permission(&clauses.owner_admin, &clauses.member, clauses.principal_col);
        let task_perm =
            build_task_permission(&clauses.owner_admin, &clauses.member, clauses.principal_col);

        let mut arms: Vec<String> = Vec::new();
        if query.owner_kind != Some(AttachmentOwnerKind::Task) {
            arms.push(build_document_arm(&document_perm, &filters));
        }
        if query.owner_kind != Some(AttachmentOwnerKind::Document) {
            arms.push(build_task_arm(&task_perm, &filters));
        }

        values.push((query.limit as i64).into());
        let limit_param = values.len();

        let union_sql = arms.join("\nUNION ALL\n");
        let sql = format!(
            "SELECT * FROM ({union_sql}) listed ORDER BY listed.id DESC LIMIT ${limit_param}"
        );

        let rows = WorkspaceAttachmentRow::find_by_statement(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            sql,
            values,
        ))
        .all(&self.conn)
        .await
        .map_err(db_err)?;

        rows.into_iter().map(row_to_workspace_attachment).collect()
    }

    async fn owner_of(
        &self,
        ctx: &WorkspaceCtx,
        id: AttachmentId,
    ) -> Result<Option<AttachmentOwnerRef>, DomainError> {
        let filters = Filters {
            conditions: "AND a.id = $2".to_string(),
        };

        let union_sql = [
            build_document_arm(ADMITS_EVERY_ROW, &filters),
            build_task_arm(ADMITS_EVERY_ROW, &filters),
        ]
        .join("\nUNION ALL\n");

        let row = WorkspaceAttachmentRow::find_by_statement(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            format!("SELECT * FROM ({union_sql}) listed LIMIT 1"),
            vec![ctx.workspace_id.0.into(), id.0.into()],
        ))
        .one(&self.conn)
        .await
        .map_err(db_err)?;

        row.map(|row| row_to_workspace_attachment(row).map(|item| item.owner))
            .transpose()
    }
}

/// The three principal-dependent SQL fragments the permission disjunctions need.
///
/// Built exactly as `PgSearchRepo::search` builds them so a row listed here is a
/// row search would list, and both mirror `permissions::resolve()`.
struct PrincipalClauses {
    owner_admin: String,
    member: String,
    principal_col: &'static str,
}

impl PrincipalClauses {
    fn build(principal: &Principal, bypass: bool, values: &mut Vec<sea_orm::Value>) -> Self {
        match principal {
            Principal::User(uid) => {
                values.push(uid.0.into());

                let owner_admin = if bypass {
                    "TRUE".to_string()
                } else {
                    "EXISTS (
                        SELECT 1 FROM acta.workspace_memberships
                        WHERE workspace_id = $1
                          AND user_id = $2
                          AND role IN ('owner', 'admin')
                    )"
                    .to_string()
                };

                Self {
                    owner_admin,
                    member: "EXISTS (
                        SELECT 1 FROM acta.workspace_memberships
                        WHERE workspace_id = $1
                          AND user_id = $2
                    )"
                    .to_string(),
                    principal_col: "user_id",
                }
            }
            Principal::ApiKey(kid) => {
                values.push(kid.0.into());
                Self {
                    owner_admin: "FALSE".to_string(),
                    member: "FALSE".to_string(),
                    principal_col: "api_key_id",
                }
            }
            Principal::Group(_) => {
                values.push(uuid::Uuid::nil().into());
                Self {
                    owner_admin: "FALSE".to_string(),
                    member: "FALSE".to_string(),
                    principal_col: "user_id",
                }
            }
        }
    }
}

struct Filters {
    conditions: String,
}

fn build_filters(query: &WorkspaceAttachmentQuery, values: &mut Vec<sea_orm::Value>) -> Filters {
    let mut conditions = String::new();

    if let Some(file_name) = query.file_name.as_deref().map(str::trim)
        && !file_name.is_empty()
    {
        let escaped = file_name
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");
        values.push(format!("%{escaped}%").into());
        conditions.push_str(&format!(
            "\n AND a.file_name ILIKE ${} ESCAPE '\\'",
            values.len()
        ));
    }

    if let Some(prefix) = query.content_type_prefix.as_deref().map(str::trim)
        && !prefix.is_empty()
    {
        values.push(prefix.to_string().into());
        conditions.push_str(&format!(
            "\n AND starts_with(a.content_type, ${})",
            values.len()
        ));
    }

    if let Some(after) = query.after {
        values.push(after.0.into());
        conditions.push_str(&format!("\n AND a.id < ${}", values.len()));
    }

    Filters { conditions }
}

/// Permission predicate for the owner lookup, which the caller authorizes itself.
const ADMITS_EVERY_ROW: &str = "TRUE";

fn build_document_arm(perm: &str, filters: &Filters) -> String {
    let live_project = project_is_live_sql("d.project_id");
    let live_folder = folder_chain_is_live_sql("d.folder_id");
    let base_where = ATTACHMENT_BASE_WHERE;
    let columns = ATTACHMENT_COLUMNS;
    let source = ATTACHMENT_SOURCE;
    let conditions = &filters.conditions;

    format!(
        r#"SELECT
            {columns},
            'document'::text AS owner_kind,
            d.title AS owner_title,
            d.id AS owner_document_id,
            d.slug AS owner_document_slug,
            NULL::uuid AS owner_task_id,
            NULL::text AS owner_task_readable_id,
            p.slug AS owner_project_slug
        {source}
        JOIN acta.documents d ON d.id = COALESCE(a.document_id, c.document_id) AND d.workspace_id = $1
        LEFT JOIN acta.projects p ON p.id = d.project_id AND p.workspace_id = $1 AND p.deleted_at IS NULL
        WHERE {base_where}
          AND d.deleted_at IS NULL
          AND ({live_project})
          AND ({live_folder})
          AND ({perm})
          {conditions}"#
    )
}

fn build_task_arm(perm: &str, filters: &Filters) -> String {
    let live_board = board_chain_is_live_sql("t.board_id");
    let base_where = ATTACHMENT_BASE_WHERE;
    let columns = ATTACHMENT_COLUMNS;
    let source = ATTACHMENT_SOURCE;
    let conditions = &filters.conditions;

    format!(
        r#"SELECT
            {columns},
            'task'::text AS owner_kind,
            t.title AS owner_title,
            NULL::uuid AS owner_document_id,
            NULL::text AS owner_document_slug,
            t.id AS owner_task_id,
            t.readable_id AS owner_task_readable_id,
            p.slug AS owner_project_slug
        {source}
        JOIN acta.tasks t ON t.id = COALESCE(a.task_id, c.task_id) AND t.workspace_id = $1
        LEFT JOIN acta.projects p ON p.id = t.project_id AND p.workspace_id = $1 AND p.deleted_at IS NULL
        WHERE {base_where}
          AND t.deleted_at IS NULL
          AND ({live_board})
          AND ({perm})
          {conditions}"#
    )
}

fn row_to_workspace_attachment(
    row: WorkspaceAttachmentRow,
) -> Result<WorkspaceAttachment, DomainError> {
    let kind = match row.owner_kind.as_str() {
        "document" => AttachmentOwnerKind::Document,
        "task" => AttachmentOwnerKind::Task,
        other => {
            return Err(DomainError::Internal {
                message: format!("unknown attachment owner kind '{other}'"),
            });
        }
    };

    let owner = AttachmentOwnerRef {
        kind,
        title: row.owner_title,
        document_id: row.owner_document_id.map(DocumentId),
        document_slug: row.owner_document_slug,
        task_id: row.owner_task_id.map(TaskId),
        task_readable_id: row.owner_task_readable_id,
        project_slug: row.owner_project_slug,
        comment_id: row.comment_id.map(CommentId),
    };

    let attachment = Attachment {
        id: AttachmentId(row.id),
        workspace_id: DomainWsId(row.workspace_id),
        document_id: row.document_id.map(DocumentId),
        task_id: row.task_id.map(TaskId),
        comment_id: row.comment_id.map(CommentId),
        draft_id: None,
        file_name: row.file_name,
        content_type: row.content_type,
        size_bytes: row.size_bytes,
        sha256: row.sha256,
        created_by_user_id: row.created_by_user_id.map(UserId),
        created_by_api_key_id: row.created_by_api_key_id.map(ApiKeyId),
        created_at: row.created_at,
        updated_at: row.updated_at,
        deleted_at: None,
    };

    Ok(WorkspaceAttachment { attachment, owner })
}
