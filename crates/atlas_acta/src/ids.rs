use atlas_core::define_id;

/// Principal identity ids are `atlas_core`-owned (D4); re-exported here so
/// Acta entities/ports that reference `UserId`/`ApiKeyId` alongside Acta ids
/// can import them from a single `crate::ids` path.
pub use atlas_core::principal::{ApiKeyId, UserId};

define_id!(WorkspaceId);
define_id!(ProjectId);
define_id!(FolderId);
define_id!(DocumentId);
define_id!(RevisionId);
define_id!(AttachmentId);
define_id!(BoardId);
define_id!(ColumnId);
define_id!(TaskId);
define_id!(TaskReferenceId);
define_id!(ChecklistItemId);
define_id!(CommentId);
define_id!(CommentDraftId);
define_id!(CommentLinkId);
define_id!(CommentLinkEventId);
define_id!(TaskActivityId);
define_id!(PropertyDefinitionId);
define_id!(MembershipId);
define_id!(TagId);
define_id!(SavedSearchId);
define_id!(TaskViewId);
define_id!(StatusTemplateId);
define_id!(PlatformStatusTemplateId);
define_id!(PurgeOperationId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_id_uses_v7_and_is_time_ordered() {
        let a = DocumentId::new();
        let b = DocumentId::new();
        assert!(b.0 > a.0, "UUIDv7 IDs must be time-ordered");
    }

    #[test]
    fn typed_ids_serialize_as_uuid_strings() {
        let id = WorkspaceId::new();
        let json = serde_json::to_string(&id).expect("serialize");
        assert!(json.starts_with('"'));
    }
}
