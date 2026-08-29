//! Breadth-first traversal of what a task links to.
//!
//! Answers "what does this task depend on, what does it block, and what is it
//! made of" in one read. The alternative — walking `get_task_references` node by
//! node — costs one round trip per node and is exactly the loop this exists to
//! collapse.

use atlas_domain::{DomainError, ids::WorkspaceId};
use sea_orm::{ColumnTrait, Condition, DatabaseConnection, EntityTrait, QueryFilter};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::persistence::entities::{
    boards_tasks::{board_column, task, task_reference},
    documents::document,
};
use atlas_postgres::db_err;

/// Traversal bounds.
///
/// A reference graph can span a whole workspace, and an agent asking about one
/// task wants its neighbourhood, not the corpus. Both bounds are enforced, and
/// hitting the node budget is reported rather than silently trimmed.
pub const MAX_DEPTH: u32 = 5;
pub const DEFAULT_DEPTH: u32 = 2;
pub const MAX_NODES: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphNodeKind {
    Task,
    Document,
}

#[derive(Debug, Clone)]
pub struct GraphNode {
    pub id: Uuid,
    pub kind: GraphNodeKind,
    pub readable_id: Option<String>,
    pub document_slug: Option<String>,
    pub title: String,
    pub column_name: Option<String>,
    pub depth: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GraphEdge {
    pub from: Uuid,
    pub to: Uuid,
    pub kind: String,
}

#[derive(Debug, Clone)]
pub struct TaskGraph {
    pub root: Uuid,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub truncated: bool,
}

/// Collects the raw graph around `root`, without permission filtering.
///
/// Filtering is the caller's job — it holds the authorization context and the
/// batch authorizer — but it must happen before the graph is returned to
/// anyone: a reference is visible to whoever can read the referencing task, and
/// its target need not be.
pub struct TaskGraphExplorer {
    conn: DatabaseConnection,
}

/// One traversal step's findings, before authorization.
pub struct GraphFrontier {
    pub tasks: Vec<Uuid>,
    pub documents: Vec<Uuid>,
    pub edges: Vec<GraphEdge>,
}

impl TaskGraphExplorer {
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Expands one level: every reference touching `frontier` in either
    /// direction, plus the parent/subtask edges those tasks carry.
    pub async fn expand(
        &self,
        workspace_id: WorkspaceId,
        frontier: &[Uuid],
    ) -> Result<GraphFrontier, DomainError> {
        let mut found = GraphFrontier {
            tasks: Vec::new(),
            documents: Vec::new(),
            edges: Vec::new(),
        };
        if frontier.is_empty() {
            return Ok(found);
        }

        let references = task_reference::Entity::find()
            .filter(task_reference::Column::WorkspaceId.eq(workspace_id.0))
            .filter(
                Condition::any()
                    .add(task_reference::Column::SourceTaskId.is_in(frontier.to_vec()))
                    .add(task_reference::Column::TargetTaskId.is_in(frontier.to_vec())),
            )
            .all(&self.conn)
            .await
            .map_err(db_err)?;

        for reference in references {
            let kind = reference.kind.clone();
            if let Some(target) = reference.target_task_id {
                found.tasks.push(target);
                found.tasks.push(reference.source_task_id);
                found.edges.push(GraphEdge {
                    from: reference.source_task_id,
                    to: target,
                    kind,
                });
            } else if let Some(target) = reference.target_document_id {
                found.documents.push(target);
                found.tasks.push(reference.source_task_id);
                found.edges.push(GraphEdge {
                    from: reference.source_task_id,
                    to: target,
                    kind,
                });
            }
        }

        let hierarchy = task::Entity::find()
            .filter(task::Column::WorkspaceId.eq(workspace_id.0))
            .filter(task::Column::DeletedAt.is_null())
            .filter(
                Condition::any()
                    .add(task::Column::Id.is_in(frontier.to_vec()))
                    .add(task::Column::ParentTaskId.is_in(frontier.to_vec())),
            )
            .all(&self.conn)
            .await
            .map_err(db_err)?;

        for row in hierarchy {
            let Some(parent_id) = row.parent_task_id else {
                continue;
            };
            found.tasks.push(row.id);
            found.tasks.push(parent_id);
            // Recorded parent → child so the direction reads the way the
            // hierarchy does, whichever end of it the traversal arrived from.
            found.edges.push(GraphEdge {
                from: parent_id,
                to: row.id,
                kind: "subtask".to_owned(),
            });
        }

        Ok(found)
    }

    /// Loads the display metadata of every collected node.
    pub async fn hydrate(
        &self,
        workspace_id: WorkspaceId,
        depths: &HashMap<Uuid, u32>,
        tasks: &[Uuid],
        documents: &[Uuid],
    ) -> Result<Vec<GraphNode>, DomainError> {
        let mut nodes = Vec::with_capacity(tasks.len() + documents.len());

        if !tasks.is_empty() {
            let rows = task::Entity::find()
                .filter(task::Column::WorkspaceId.eq(workspace_id.0))
                .filter(task::Column::DeletedAt.is_null())
                .filter(task::Column::Id.is_in(tasks.to_vec()))
                .all(&self.conn)
                .await
                .map_err(db_err)?;

            let column_ids: Vec<Uuid> = rows.iter().map(|row| row.column_id).collect();
            let columns: HashMap<Uuid, String> = board_column::Entity::find()
                .filter(board_column::Column::WorkspaceId.eq(workspace_id.0))
                .filter(board_column::Column::DeletedAt.is_null())
                .filter(board_column::Column::Id.is_in(column_ids))
                .all(&self.conn)
                .await
                .map_err(db_err)?
                .into_iter()
                .map(|column| (column.id, column.name))
                .collect();

            for row in rows {
                let column_name = columns.get(&row.column_id).cloned();
                nodes.push(GraphNode {
                    depth: depths.get(&row.id).copied().unwrap_or(0),
                    id: row.id,
                    kind: GraphNodeKind::Task,
                    readable_id: Some(row.readable_id),
                    document_slug: None,
                    title: row.title,
                    column_name,
                });
            }
        }

        if !documents.is_empty() {
            let rows = document::Entity::find()
                .filter(document::Column::WorkspaceId.eq(workspace_id.0))
                .filter(document::Column::DeletedAt.is_null())
                .filter(document::Column::Id.is_in(documents.to_vec()))
                .all(&self.conn)
                .await
                .map_err(db_err)?;

            for row in rows {
                nodes.push(GraphNode {
                    depth: depths.get(&row.id).copied().unwrap_or(0),
                    id: row.id,
                    kind: GraphNodeKind::Document,
                    readable_id: None,
                    document_slug: row.slug,
                    title: row.title,
                    column_name: None,
                });
            }
        }

        nodes.sort_by(|a, b| {
            a.depth
                .cmp(&b.depth)
                .then_with(|| a.readable_id.cmp(&b.readable_id))
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(nodes)
    }
}

/// Drops edges whose endpoints did not survive authorization or hydration.
///
/// An edge to a node the caller cannot read would leak that the resource exists
/// and how it relates to a task they can see.
pub fn retain_edges_within(edges: Vec<GraphEdge>, visible: &HashSet<Uuid>) -> Vec<GraphEdge> {
    let mut kept: Vec<GraphEdge> = edges
        .into_iter()
        .filter(|edge| visible.contains(&edge.from) && visible.contains(&edge.to))
        .collect();

    kept.sort_by(|a, b| {
        a.from
            .cmp(&b.from)
            .then_with(|| a.to.cmp(&b.to))
            .then_with(|| a.kind.cmp(&b.kind))
    });
    kept.dedup();
    kept
}

/// Clamps a requested traversal depth into the supported range.
pub fn resolve_depth(requested: Option<u32>) -> u32 {
    requested.unwrap_or(DEFAULT_DEPTH).clamp(1, MAX_DEPTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(from: u128, to: u128) -> GraphEdge {
        GraphEdge {
            from: Uuid::from_u128(from),
            to: Uuid::from_u128(to),
            kind: "blocks".to_owned(),
        }
    }

    #[test]
    fn depth_is_clamped_into_the_supported_range() {
        assert_eq!(resolve_depth(None), DEFAULT_DEPTH);
        assert_eq!(resolve_depth(Some(0)), 1);
        assert_eq!(resolve_depth(Some(99)), MAX_DEPTH);
        assert_eq!(resolve_depth(Some(3)), 3);
    }

    #[test]
    fn an_edge_to_an_invisible_node_is_dropped_whole() {
        let visible: HashSet<Uuid> = [Uuid::from_u128(1), Uuid::from_u128(2)]
            .into_iter()
            .collect();

        let kept = retain_edges_within(vec![edge(1, 2), edge(1, 3), edge(4, 1)], &visible);

        assert_eq!(kept, vec![edge(1, 2)]);
    }

    #[test]
    fn duplicate_edges_collapse() {
        let visible: HashSet<Uuid> = [Uuid::from_u128(1), Uuid::from_u128(2)]
            .into_iter()
            .collect();

        let kept = retain_edges_within(vec![edge(1, 2), edge(1, 2)], &visible);

        assert_eq!(kept.len(), 1);
    }
}
