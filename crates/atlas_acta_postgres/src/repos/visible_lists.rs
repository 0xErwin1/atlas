//! V2 list reads for projects, folders, boards and tasks: the workspace's
//! live rows that a V2 list visibility predicate selects, filtered inside
//! WHERE so LIMIT and the id cursor only ever see visible rows (ACTA-AUTHZ-3,
//! EVAL-5). Additive: no route calls them until the V2 cutover; the V1 lists
//! and their gates are unchanged. Documents have their V2 variant beside the
//! V1 list on `PgDocumentRepo`.

use atlas_acta::actor::WorkspaceCtx;
use atlas_acta::entities::boards_tasks::{Board, Task};
use atlas_acta::entities::workspace_core::{Folder, Project};
use atlas_acta::ids::{BoardId, ProjectId};
use atlas_core::error::DomainError;
use atlas_core::visibility::ListVisibility;
use atlas_postgres::db_err;
use sea_orm::{DatabaseBackend, DatabaseConnection, EntityTrait, FromQueryResult, Statement};
use uuid::Uuid;

use crate::entities::boards_tasks::{board, board_from, task, task_from};
use crate::entities::workspace_core::{folder, folder_from, project, project_from};
use crate::live_ancestors::{
    board_chain_is_live_sql, folder_chain_is_live_sql, project_is_live_sql, task_chain_is_live_sql,
};
use crate::visibility_sql::{VisibleKind, predicate_to_sql};

pub struct PgVisibleListRepo {
    pub conn: DatabaseConnection,
}

/// The alias every V2 list gives its row.
const LISTED: &str = "listed";

/// One page of a V2 list: the rows after `after_id`, at most `limit`, in id
/// order.
#[derive(Debug, Clone, Copy)]
pub struct Page {
    pub after_id: Option<Uuid>,
    pub limit: u64,
}

impl PgVisibleListRepo {
    /// Live projects of the workspace that `visibility` (for
    /// `acta::project::read`) selects.
    pub async fn list_projects_v2(
        &self,
        ctx: &WorkspaceCtx,
        visibility: &ListVisibility,
        page: Page,
    ) -> Result<Vec<Project>, DomainError> {
        let query = ListQuery {
            kind: VisibleKind::Project,
            table: "projects",
            liveness: "TRUE".to_string(),
            filters: Vec::new(),
        };

        let rows = self
            .rows::<project::Entity>(ctx, visibility, query, page)
            .await?;

        Ok(rows.into_iter().map(project_from).collect())
    }

    /// Live folders of the workspace, optionally of one project, that
    /// `visibility` (for `acta::folder::read`) selects.
    pub async fn list_folders_v2(
        &self,
        ctx: &WorkspaceCtx,
        visibility: &ListVisibility,
        project: Option<ProjectId>,
        page: Page,
    ) -> Result<Vec<Folder>, DomainError> {
        let query = ListQuery {
            kind: VisibleKind::Folder,
            table: "folders",
            liveness: format!(
                "{} AND {}",
                project_is_live_sql(&format!("{LISTED}.project_id")),
                folder_chain_is_live_sql(&format!("{LISTED}.id")),
            ),
            filters: project.map(|id| ("project_id", id.0)).into_iter().collect(),
        };

        let rows = self
            .rows::<folder::Entity>(ctx, visibility, query, page)
            .await?;

        Ok(rows.into_iter().map(folder_from).collect())
    }

    /// Live boards of the workspace, optionally of one project, that
    /// `visibility` (for `acta::board::read`) selects.
    pub async fn list_boards_v2(
        &self,
        ctx: &WorkspaceCtx,
        visibility: &ListVisibility,
        project: Option<ProjectId>,
        page: Page,
    ) -> Result<Vec<Board>, DomainError> {
        let query = ListQuery {
            kind: VisibleKind::Board,
            table: "boards",
            liveness: board_chain_is_live_sql(&format!("{LISTED}.id")),
            filters: project.map(|id| ("project_id", id.0)).into_iter().collect(),
        };

        let rows = self
            .rows::<board::Entity>(ctx, visibility, query, page)
            .await?;

        Ok(rows.into_iter().map(board_from).collect())
    }

    /// Live top-level tasks of the workspace, optionally of one board, that
    /// `visibility` (for `acta::task::read`) selects. Subtasks are left out,
    /// as the V1 workspace task list leaves them out.
    pub async fn list_tasks_v2(
        &self,
        ctx: &WorkspaceCtx,
        visibility: &ListVisibility,
        board: Option<BoardId>,
        page: Page,
    ) -> Result<Vec<Task>, DomainError> {
        let query = ListQuery {
            kind: VisibleKind::Task,
            table: "tasks",
            liveness: format!(
                "{LISTED}.parent_task_id IS NULL AND {}",
                task_chain_is_live_sql(&format!("{LISTED}.id")),
            ),
            filters: board.map(|id| ("board_id", id.0)).into_iter().collect(),
        };

        let rows = self
            .rows::<task::Entity>(ctx, visibility, query, page)
            .await?;

        Ok(rows.into_iter().map(task_from).collect())
    }

    /// The rows of `query.table` in the workspace that are not deleted,
    /// pass `query.liveness` and the equality `query.filters`, and that
    /// `visibility` selects, one page in id order.
    async fn rows<E>(
        &self,
        ctx: &WorkspaceCtx,
        visibility: &ListVisibility,
        query: ListQuery,
        page: Page,
    ) -> Result<Vec<E::Model>, DomainError>
    where
        E: EntityTrait,
        E::Model: FromQueryResult,
    {
        let mut values: Vec<sea_orm::Value> = vec![ctx.workspace_id.0.into()];
        let fragment = predicate_to_sql(visibility, query.kind, LISTED, values.len() + 1);
        values.extend(fragment.binds);

        let mut conditions: Vec<String> = Vec::new();
        for (column, id) in query.filters {
            values.push(id.into());
            conditions.push(format!("AND {LISTED}.{column} = ${}", values.len()));
        }
        if let Some(after_id) = page.after_id {
            values.push(after_id.into());
            conditions.push(format!("AND {LISTED}.id > ${}", values.len()));
        }

        let sql = format!(
            "SELECT {LISTED}.* FROM acta.{table} {LISTED} \
             WHERE {LISTED}.workspace_id = $1 \
               AND {LISTED}.deleted_at IS NULL \
               AND {liveness} \
               AND {visible} \
               {conditions} \
             ORDER BY {LISTED}.id \
             LIMIT {limit}",
            table = query.table,
            liveness = query.liveness,
            visible = fragment.where_clause,
            conditions = conditions.join(" "),
            limit = page.limit,
        );

        E::find()
            .from_raw_sql(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                sql,
                values,
            ))
            .all(&self.conn)
            .await
            .map_err(db_err)
    }
}

/// What distinguishes one kind's V2 list: its table, its liveness beyond
/// its own `deleted_at`, and its optional equality filters.
struct ListQuery {
    kind: VisibleKind,
    table: &'static str,
    liveness: String,
    filters: Vec<(&'static str, Uuid)>,
}
