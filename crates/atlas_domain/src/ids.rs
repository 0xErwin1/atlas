use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

macro_rules! define_id {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl From<Uuid> for $name {
            fn from(id: Uuid) -> Self {
                Self(id)
            }
        }
    };
}

define_id!(WorkspaceId);
define_id!(UserId);
define_id!(ApiKeyId);
define_id!(SessionId);
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
define_id!(TaskActivityId);
define_id!(PropertyDefinitionId);
define_id!(MembershipId);
define_id!(TagId);
define_id!(SavedSearchId);
define_id!(TaskViewId);

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
