//! Bridges the V1 authorization `ResourceRef` enum to `atlas_core::ids::ResourceRef`,
//! the `acta::<kind>::<hyphenated-uuid>` string encoding that S3c persists.
//!
//! `ResourceRef::Workspace` carries no id of its own — the workspace is
//! supplied out-of-band by the authorization context — so `from_core` must be
//! given the same workspace to decode against. A foreign workspace id in the
//! decoded string is rejected rather than silently accepted, because silently
//! accepting it would let S3c read a cross-workspace grant as if it were
//! scoped to the caller's own workspace.

use crate::error::DomainError;
use crate::ids::WorkspaceId;
use crate::permissions::ResourceRef;

const PRODUCT: &str = "acta";

/// Converts a V1 `ResourceRef` to the canonical `atlas_core::ids::ResourceRef`
/// string encoding. Infallible: every segment is already validated (`acta`,
/// a fixed kind literal, and a UUID `Display` id contain no reserved chars).
pub fn to_core(resource: &ResourceRef, workspace: WorkspaceId) -> atlas_core::ids::ResourceRef {
    let (kind, id) = match resource {
        ResourceRef::Workspace => ("workspace", workspace.0),
        ResourceRef::Project(id) => ("project", id.0),
        ResourceRef::Folder(id) => ("folder", id.0),
        ResourceRef::Document(id) => ("document", id.0),
        ResourceRef::Board(id) => ("board", id.0),
    };

    match atlas_core::ids::ResourceRef::new(PRODUCT, kind, &id.to_string()) {
        Ok(resource_ref) => resource_ref,
        Err(_) => unreachable!(
            "the acta product, a fixed kind literal, and a hyphenated uuid id are always valid segments"
        ),
    }
}

/// Converts a canonical `atlas_core::ids::ResourceRef` back to a V1
/// `ResourceRef`, scoped to `workspace`.
///
/// Fails when the product segment is not `acta`, the kind segment is not one
/// of the five known kinds, the id segment is not a valid UUID, or the
/// decoded kind is `workspace` and its id does not equal `workspace`.
pub fn from_core(
    resource: &atlas_core::ids::ResourceRef,
    workspace: WorkspaceId,
) -> Result<ResourceRef, DomainError> {
    if resource.product() != PRODUCT {
        return Err(DomainError::InvalidInput {
            message: format!("unknown resource ref product: {}", resource.product()),
        });
    }

    let id = resource
        .id()
        .parse::<::uuid::Uuid>()
        .map_err(|_| DomainError::InvalidInput {
            message: format!("resource ref id is not a valid uuid: {}", resource.id()),
        })?;

    match resource.kind() {
        "workspace" if id == workspace.0 => Ok(ResourceRef::Workspace),
        "workspace" => Err(DomainError::InvalidInput {
            message: "resource ref workspace id does not match the calling workspace".into(),
        }),
        "project" => Ok(ResourceRef::Project(crate::ids::ProjectId(id))),
        "folder" => Ok(ResourceRef::Folder(crate::ids::FolderId(id))),
        "document" => Ok(ResourceRef::Document(crate::ids::DocumentId(id))),
        "board" => Ok(ResourceRef::Board(crate::ids::BoardId(id))),
        other => Err(DomainError::InvalidInput {
            message: format!("unknown resource ref kind: {other}"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{BoardId, DocumentId, FolderId, ProjectId};
    use uuid::Uuid;

    fn workspace() -> WorkspaceId {
        WorkspaceId(Uuid::now_v7())
    }

    #[test]
    fn round_trips_workspace_through_the_canonical_golden_string() {
        let workspace = workspace();

        let core = to_core(&ResourceRef::Workspace, workspace);
        assert_eq!(
            core.to_string(),
            format!("acta::workspace::{}", workspace.0)
        );

        let decoded = from_core(&core, workspace).expect("workspace round trip");
        assert_eq!(decoded, ResourceRef::Workspace);
    }

    #[test]
    fn round_trips_project_through_the_canonical_golden_string() {
        let workspace = workspace();
        let project = ProjectId(Uuid::now_v7());

        let core = to_core(&ResourceRef::Project(project), workspace);
        assert_eq!(core.to_string(), format!("acta::project::{}", project.0));

        let decoded = from_core(&core, workspace).expect("project round trip");
        assert_eq!(decoded, ResourceRef::Project(project));
    }

    #[test]
    fn round_trips_folder_through_the_canonical_golden_string() {
        let workspace = workspace();
        let folder = FolderId(Uuid::now_v7());

        let core = to_core(&ResourceRef::Folder(folder), workspace);
        assert_eq!(core.to_string(), format!("acta::folder::{}", folder.0));

        let decoded = from_core(&core, workspace).expect("folder round trip");
        assert_eq!(decoded, ResourceRef::Folder(folder));
    }

    #[test]
    fn round_trips_document_through_the_canonical_golden_string() {
        let workspace = workspace();
        let document = DocumentId(Uuid::now_v7());

        let core = to_core(&ResourceRef::Document(document), workspace);
        assert_eq!(core.to_string(), format!("acta::document::{}", document.0));

        let decoded = from_core(&core, workspace).expect("document round trip");
        assert_eq!(decoded, ResourceRef::Document(document));
    }

    #[test]
    fn round_trips_board_through_the_canonical_golden_string() {
        let workspace = workspace();
        let board = BoardId(Uuid::now_v7());

        let core = to_core(&ResourceRef::Board(board), workspace);
        assert_eq!(core.to_string(), format!("acta::board::{}", board.0));

        let decoded = from_core(&core, workspace).expect("board round trip");
        assert_eq!(decoded, ResourceRef::Board(board));
    }

    #[test]
    fn from_core_rejects_a_foreign_workspace_id() {
        let context_workspace = workspace();
        let foreign_workspace = workspace();
        let core: atlas_core::ids::ResourceRef =
            format!("acta::workspace::{}", foreign_workspace.0)
                .parse()
                .expect("valid core resource ref");

        let result = from_core(&core, context_workspace);

        let message = invalid_input_message(result);
        assert_eq!(
            message,
            "resource ref workspace id does not match the calling workspace"
        );
    }

    #[test]
    fn from_core_rejects_an_unknown_kind_segment() {
        let workspace = workspace();
        let core: atlas_core::ids::ResourceRef = format!("acta::user::{}", Uuid::now_v7())
            .parse()
            .expect("valid core resource ref");

        let message = invalid_input_message(from_core(&core, workspace));
        assert_eq!(message, "unknown resource ref kind: user");
    }

    #[test]
    fn from_core_rejects_an_unknown_product_segment() {
        let workspace = workspace();
        let core: atlas_core::ids::ResourceRef = format!("other::workspace::{}", workspace.0)
            .parse()
            .expect("valid core resource ref");

        let message = invalid_input_message(from_core(&core, workspace));
        assert_eq!(message, "unknown resource ref product: other");
    }

    #[test]
    fn from_core_rejects_a_non_uuid_id_segment() {
        let workspace = workspace();
        let core: atlas_core::ids::ResourceRef = "acta::workspace::not-a-uuid"
            .parse()
            .expect("valid core resource ref");

        let message = invalid_input_message(from_core(&core, workspace));
        assert_eq!(message, "resource ref id is not a valid uuid: not-a-uuid");
    }

    fn invalid_input_message(result: Result<ResourceRef, DomainError>) -> String {
        match result {
            Err(DomainError::InvalidInput { message }) => message,
            other => panic!("expected DomainError::InvalidInput, got {other:?}"),
        }
    }
}
