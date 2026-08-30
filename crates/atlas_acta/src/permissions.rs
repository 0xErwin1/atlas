use crate::entities::boards_tasks::ReferenceKind;
use crate::ids::{BoardId, DocumentId, FolderId, ProjectId, TaskId};
use atlas_core::error::DomainError;
use serde::{Deserialize, Serialize};

pub mod resource_ref_codec;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResourceRef {
    Workspace,
    Project(ProjectId),
    Folder(FolderId),
    Document(DocumentId),
    Board(BoardId),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VisibilityRole {
    Viewer,
    Editor,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
    Private,
    Workspace(VisibilityRole),
    Public(VisibilityRole),
}

/// Validates that a task reference has exactly one target consistent with its kind,
/// and that the source task does not reference itself.
///
/// Spec/Docs → document target; Relates/Blocks/Parent → task target.
/// Multi-node Parent cycles (A→B→A) are not detected here; they require DB
/// ancestry traversal and are left as a follow-up.
pub fn validate_reference(
    source_task_id: TaskId,
    kind: ReferenceKind,
    target_task_id: Option<TaskId>,
    target_document_id: Option<DocumentId>,
) -> Result<(), DomainError> {
    match (target_task_id, target_document_id) {
        (Some(_), Some(_)) => {
            return Err(DomainError::InvalidInput {
                message: "a task reference must have exactly one target, not both".into(),
            });
        }
        (None, None) => {
            return Err(DomainError::InvalidInput {
                message: "a task reference must have exactly one target".into(),
            });
        }
        _ => {}
    }

    if target_task_id == Some(source_task_id) {
        return Err(DomainError::InvalidInput {
            message: "a task cannot reference itself".into(),
        });
    }

    match kind {
        ReferenceKind::Spec | ReferenceKind::Docs => {
            if target_document_id.is_none() {
                return Err(DomainError::InvalidInput {
                    message: format!("{} reference requires a document target", kind.as_str()),
                });
            }
        }
        ReferenceKind::Relates | ReferenceKind::Blocks | ReferenceKind::Parent => {
            if target_task_id.is_none() {
                return Err(DomainError::InvalidInput {
                    message: format!("{} reference requires a task target", kind.as_str()),
                });
            }
        }
    }

    Ok(())
}
