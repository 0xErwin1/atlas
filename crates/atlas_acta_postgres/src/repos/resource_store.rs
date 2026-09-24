//! Postgres side of Acta's resource provider: each row's liveness and its
//! path parent, read by id per kind, and the user ids behind a workspace's
//! `members` set. Path assembly and chain liveness stay in
//! `atlas_acta::provider`.

use async_trait::async_trait;
use atlas_acta::provider::{ActaKind, ActaNode, ActaResourceStore};
use atlas_core::error::DomainError;
use atlas_postgres::db_err;
use sea_orm::{DatabaseBackend, DatabaseConnection, FromQueryResult, Statement, Value};
use uuid::Uuid;

pub struct PgActaResourceStore {
    pub conn: DatabaseConnection,
}

#[derive(Debug, FromQueryResult)]
struct NodeRow {
    id: Uuid,
    live: bool,
    parent_kind: Option<String>,
    parent_id: Option<Uuid>,
}

#[derive(Debug, FromQueryResult)]
struct IdRow {
    id: Uuid,
}

/// The table behind a kind and the SQL expressions naming its path parent's
/// kind and id, or `None` for a kind without rows. A folder or document hangs
/// off its folder, else its project, else its workspace; a board off its
/// folder, else its project; a comment off its task, else its document; an
/// attachment off its comment, document or task (a draft-only attachment has
/// no parent). Subtasks hang off their own board like any task.
fn node_source(kind: ActaKind) -> Option<(&'static str, &'static str, &'static str)> {
    let workspace_child = ("'workspace'", "workspace_id");

    let (table, (parent_kind, parent_id)) = match kind {
        ActaKind::Workspace => ("acta.workspaces", ("NULL", "NULL::uuid")),
        ActaKind::Project => ("acta.projects", workspace_child),
        ActaKind::Folder => (
            "acta.folders",
            (
                "CASE WHEN parent_folder_id IS NOT NULL THEN 'folder' \
                      WHEN project_id IS NOT NULL THEN 'project' ELSE 'workspace' END",
                "COALESCE(parent_folder_id, project_id, workspace_id)",
            ),
        ),
        ActaKind::Document => (
            "acta.documents",
            (
                "CASE WHEN folder_id IS NOT NULL THEN 'folder' \
                      WHEN project_id IS NOT NULL THEN 'project' ELSE 'workspace' END",
                "COALESCE(folder_id, project_id, workspace_id)",
            ),
        ),
        ActaKind::Board => (
            "acta.boards",
            (
                "CASE WHEN folder_id IS NOT NULL THEN 'folder' ELSE 'project' END",
                "COALESCE(folder_id, project_id)",
            ),
        ),
        ActaKind::Column => ("acta.board_columns", ("'board'", "board_id")),
        ActaKind::Task => ("acta.tasks", ("'board'", "board_id")),
        ActaKind::ChecklistItem => ("acta.task_checklist_items", ("'task'", "task_id")),
        ActaKind::Comment => (
            "acta.comments",
            (
                "CASE WHEN task_id IS NOT NULL THEN 'task' \
                      WHEN document_id IS NOT NULL THEN 'document' END",
                "COALESCE(task_id, document_id)",
            ),
        ),
        ActaKind::Attachment => (
            "acta.attachments",
            (
                "CASE WHEN comment_id IS NOT NULL THEN 'comment' \
                      WHEN document_id IS NOT NULL THEN 'document' \
                      WHEN task_id IS NOT NULL THEN 'task' END",
                "COALESCE(comment_id, document_id, task_id)",
            ),
        ),
        ActaKind::SavedSearch => ("acta.saved_searches", workspace_child),
        ActaKind::TaskView => ("acta.task_views", workspace_child),
        ActaKind::Tag => ("acta.tags", workspace_child),
        ActaKind::PropertyDefinition => ("acta.property_definitions", workspace_child),
        ActaKind::StatusTemplate => ("acta.workspace_status_templates", workspace_child),
        ActaKind::Webhook => ("acta.webhook_subscriptions", workspace_child),
        ActaKind::AutomationRule => ("acta.automation_rules", workspace_child),
        ActaKind::IntegrationConfig => ("acta.integration_configs", workspace_child),
        ActaKind::ShareLink => return None,
    };

    Some((table, parent_kind, parent_id))
}

/// `$1, $2, ...` for `count` bound values.
fn placeholders(count: usize) -> String {
    (1..=count)
        .map(|index| format!("${index}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn node_from(row: NodeRow) -> ActaNode {
    let parent = row
        .parent_kind
        .as_deref()
        .and_then(ActaKind::parse)
        .zip(row.parent_id);

    ActaNode {
        id: row.id,
        live: row.live,
        parent,
    }
}

#[async_trait]
impl ActaResourceStore for PgActaResourceStore {
    /// One query for every id of `kind`; ids without a row are absent from
    /// the result.
    async fn nodes(&self, kind: ActaKind, ids: &[Uuid]) -> Result<Vec<ActaNode>, DomainError> {
        let Some((table, parent_kind, parent_id)) = node_source(kind) else {
            return Ok(Vec::new());
        };
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        let sql = format!(
            "SELECT id, deleted_at IS NULL AS live, {parent_kind} AS parent_kind, \
                 {parent_id} AS parent_id \
             FROM {table} WHERE id IN ({})",
            placeholders(ids.len())
        );
        let values: Vec<Value> = ids.iter().map(|id| (*id).into()).collect();

        let rows = NodeRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            sql,
            values,
        ))
        .all(&self.conn)
        .await
        .map_err(db_err)?;

        Ok(rows.into_iter().map(node_from).collect())
    }

    /// `None` when the workspace has no live row; otherwise every membership's
    /// user, whatever its role, ordered by id.
    async fn workspace_members(&self, workspace: Uuid) -> Result<Option<Vec<Uuid>>, DomainError> {
        let live = IdRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT id FROM acta.workspaces WHERE id = $1 AND deleted_at IS NULL",
            [workspace.into()],
        ))
        .one(&self.conn)
        .await
        .map_err(db_err)?;
        if live.is_none() {
            return Ok(None);
        }

        let members = IdRow::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT DISTINCT user_id AS id FROM acta.workspace_memberships \
             WHERE workspace_id = $1 ORDER BY id",
            [workspace.into()],
        ))
        .all(&self.conn)
        .await
        .map_err(db_err)?;

        Ok(Some(members.into_iter().map(|row| row.id).collect()))
    }
}
