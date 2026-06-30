#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use crate::{
    actor::Actor,
    ids::{BoardId, ColumnId, DocumentId, FolderId, ProjectId, RevisionId, TaskId, WorkspaceId},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Integer version constants per event type. Additive field additions do not
/// bump the version; only breaking schema changes do.
pub mod version {
    pub const TASK_CREATED: i32 = 1;
    pub const TASK_UPDATED: i32 = 1;
    pub const TASK_MOVED: i32 = 1;
    pub const TASK_DELETED: i32 = 1;
    pub const DOCUMENT_CREATED: i32 = 1;
    pub const DOCUMENT_UPDATED: i32 = 1;
    pub const DOCUMENT_MOVED: i32 = 1;
    pub const DOCUMENT_DELETED: i32 = 1;
    pub const BOARD_CREATED: i32 = 1;
    pub const BOARD_DELETED: i32 = 1;
    pub const COLUMN_CREATED: i32 = 1;
    pub const COLUMN_DELETED: i32 = 1;
    pub const FOLDER_CREATED: i32 = 1;
    pub const FOLDER_DELETED: i32 = 1;
}

/// Identifies the origin channel of a domain event.
///
/// Only `Internal` is emitted by this system today. `External` is a
/// forward-compatibility placeholder for future integration sources
/// (e.g. `"external/github"`); deserializing unknown strings produces
/// `External(...)` rather than an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventSource {
    Internal,
    External(String),
}

impl Serialize for EventSource {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let s = match self {
            EventSource::Internal => "internal",
            EventSource::External(s) => s.as_str(),
        };
        serializer.serialize_str(s)
    }
}

impl<'de> Deserialize<'de> for EventSource {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(if s == "internal" {
            EventSource::Internal
        } else {
            EventSource::External(s)
        })
    }
}

/// Discriminates the principal kind in a serialized event actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventActorType {
    User,
    ApiKey,
}

/// Wire representation of the acting principal carried in every `EventEnvelope`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventActor {
    #[serde(rename = "type")]
    pub actor_type: EventActorType,
    pub id: Uuid,
}

impl From<Actor> for EventActor {
    fn from(actor: Actor) -> Self {
        match actor {
            Actor::User(uid) => EventActor {
                actor_type: EventActorType::User,
                id: uid.0,
            },
            Actor::ApiKey(kid) => EventActor {
                actor_type: EventActorType::ApiKey,
                id: kid.0,
            },
        }
    }
}

// ─── Per-type payload structs ────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskCreatedPayload {
    pub task_id: TaskId,
    pub title: String,
    pub project_id: ProjectId,
    pub board_id: BoardId,
    pub column_id: ColumnId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskUpdatedPayload {
    pub task_id: TaskId,
    pub changed_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskMovedPayload {
    pub task_id: TaskId,
    pub from_column_id: ColumnId,
    pub to_column_id: ColumnId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskDeletedPayload {
    pub task_id: TaskId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentCreatedPayload {
    pub document_id: DocumentId,
    pub slug: String,
    pub title: String,
    pub project_id: Option<ProjectId>,
    pub folder_id: Option<FolderId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentUpdatedPayload {
    pub document_id: DocumentId,
    pub revision_id: RevisionId,
    pub seq: i64,
}

/// `deny_unknown_fields` ensures this minimal struct does not greedily match
/// `DocumentMovedPayload` (which has all-optional extra fields) during
/// untagged enum deserialization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentDeletedPayload {
    pub document_id: DocumentId,
}

/// Fields `from_folder_id`, `to_folder_id`, and `project_id` are optional
/// because a document may reside at the project root (no folder) and a
/// document may not belong to any project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentMovedPayload {
    pub document_id: DocumentId,
    pub from_folder_id: Option<FolderId>,
    pub to_folder_id: Option<FolderId>,
    pub project_id: Option<ProjectId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoardCreatedPayload {
    pub board_id: BoardId,
    pub project_id: ProjectId,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardDeletedPayload {
    pub board_id: BoardId,
    pub project_id: ProjectId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColumnCreatedPayload {
    pub board_id: BoardId,
    pub column_id: ColumnId,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColumnDeletedPayload {
    pub board_id: BoardId,
    pub column_id: ColumnId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FolderCreatedPayload {
    pub folder_id: FolderId,
    pub project_id: ProjectId,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FolderDeletedPayload {
    pub folder_id: FolderId,
    pub project_id: ProjectId,
}

// ─── DomainEvent enum ────────────────────────────────────────────────────────

/// Typed catalog of all domain events emitted by this system.
///
/// Serializes as the inner payload (untagged); the `event_type` string is
/// conveyed separately in `EventEnvelope` so the `data` object in the wire
/// body contains only payload fields.
///
/// Variant ordering within categories matters for untagged deserialization:
/// more-specific variants (more required fields) come before minimal ones so
/// that the first match is always the correct one. `deny_unknown_fields` on
/// minimal deleted payloads guards against false positives when optional-field
/// variants share the same primary discriminating key (e.g. `document_id`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DomainEvent {
    // Task events
    TaskCreated(TaskCreatedPayload),
    TaskUpdated(TaskUpdatedPayload),
    TaskMoved(TaskMovedPayload),
    TaskDeleted(TaskDeletedPayload),
    // Document events — deleted before moved so the minimal struct is tried first
    DocumentCreated(DocumentCreatedPayload),
    DocumentUpdated(DocumentUpdatedPayload),
    DocumentDeleted(DocumentDeletedPayload),
    DocumentMoved(DocumentMovedPayload),
    // Board events
    BoardCreated(BoardCreatedPayload),
    BoardDeleted(BoardDeletedPayload),
    // Column events
    ColumnCreated(ColumnCreatedPayload),
    ColumnDeleted(ColumnDeletedPayload),
    // Folder events
    FolderCreated(FolderCreatedPayload),
    FolderDeleted(FolderDeletedPayload),
}

impl DomainEvent {
    /// Returns the stable `event_type` string used in the envelope and outbox row.
    pub fn event_type(&self) -> &'static str {
        match self {
            DomainEvent::TaskCreated(_) => "task.created",
            DomainEvent::TaskUpdated(_) => "task.updated",
            DomainEvent::TaskMoved(_) => "task.moved",
            DomainEvent::TaskDeleted(_) => "task.deleted",
            DomainEvent::DocumentCreated(_) => "document.created",
            DomainEvent::DocumentUpdated(_) => "document.updated",
            DomainEvent::DocumentMoved(_) => "document.moved",
            DomainEvent::DocumentDeleted(_) => "document.deleted",
            DomainEvent::BoardCreated(_) => "board.created",
            DomainEvent::BoardDeleted(_) => "board.deleted",
            DomainEvent::ColumnCreated(_) => "column.created",
            DomainEvent::ColumnDeleted(_) => "column.deleted",
            DomainEvent::FolderCreated(_) => "folder.created",
            DomainEvent::FolderDeleted(_) => "folder.deleted",
        }
    }

    /// Returns the integer schema version for this variant.
    pub fn event_version(&self) -> i32 {
        match self {
            DomainEvent::TaskCreated(_) => version::TASK_CREATED,
            DomainEvent::TaskUpdated(_) => version::TASK_UPDATED,
            DomainEvent::TaskMoved(_) => version::TASK_MOVED,
            DomainEvent::TaskDeleted(_) => version::TASK_DELETED,
            DomainEvent::DocumentCreated(_) => version::DOCUMENT_CREATED,
            DomainEvent::DocumentUpdated(_) => version::DOCUMENT_UPDATED,
            DomainEvent::DocumentMoved(_) => version::DOCUMENT_MOVED,
            DomainEvent::DocumentDeleted(_) => version::DOCUMENT_DELETED,
            DomainEvent::BoardCreated(_) => version::BOARD_CREATED,
            DomainEvent::BoardDeleted(_) => version::BOARD_DELETED,
            DomainEvent::ColumnCreated(_) => version::COLUMN_CREATED,
            DomainEvent::ColumnDeleted(_) => version::COLUMN_DELETED,
            DomainEvent::FolderCreated(_) => version::FOLDER_CREATED,
            DomainEvent::FolderDeleted(_) => version::FOLDER_DELETED,
        }
    }

    /// Returns the aggregate type string for scope-matching in outbox delivery.
    pub fn aggregate_type(&self) -> &'static str {
        match self {
            DomainEvent::TaskCreated(_)
            | DomainEvent::TaskUpdated(_)
            | DomainEvent::TaskMoved(_)
            | DomainEvent::TaskDeleted(_) => "task",
            DomainEvent::DocumentCreated(_)
            | DomainEvent::DocumentUpdated(_)
            | DomainEvent::DocumentMoved(_)
            | DomainEvent::DocumentDeleted(_) => "document",
            DomainEvent::BoardCreated(_) | DomainEvent::BoardDeleted(_) => "board",
            DomainEvent::ColumnCreated(_) | DomainEvent::ColumnDeleted(_) => "column",
            DomainEvent::FolderCreated(_) | DomainEvent::FolderDeleted(_) => "folder",
        }
    }

    /// Returns the primary aggregate UUID carried by this event.
    pub fn aggregate_id(&self) -> Uuid {
        match self {
            DomainEvent::TaskCreated(p) => p.task_id.0,
            DomainEvent::TaskUpdated(p) => p.task_id.0,
            DomainEvent::TaskMoved(p) => p.task_id.0,
            DomainEvent::TaskDeleted(p) => p.task_id.0,
            DomainEvent::DocumentCreated(p) => p.document_id.0,
            DomainEvent::DocumentUpdated(p) => p.document_id.0,
            DomainEvent::DocumentMoved(p) => p.document_id.0,
            DomainEvent::DocumentDeleted(p) => p.document_id.0,
            DomainEvent::BoardCreated(p) => p.board_id.0,
            DomainEvent::BoardDeleted(p) => p.board_id.0,
            DomainEvent::ColumnCreated(p) => p.column_id.0,
            DomainEvent::ColumnDeleted(p) => p.column_id.0,
            DomainEvent::FolderCreated(p) => p.folder_id.0,
            DomainEvent::FolderDeleted(p) => p.folder_id.0,
        }
    }
}

// ─── EventEnvelope ───────────────────────────────────────────────────────────

/// The versioned wire envelope POSTed to webhook subscribers and stored as a
/// single JSONB blob in the `events_outbox` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    /// UUIDv7 idempotency key; unique per emitted event.
    pub id: Uuid,
    /// Stable dot-separated event type string (e.g. `"task.created"`).
    pub event_type: String,
    /// Schema version. Additive changes do not bump this; subscribers must
    /// tolerate unknown fields (per the contract).
    pub version: i32,
    pub source: EventSource,
    pub workspace_id: WorkspaceId,
    pub project_id: Option<ProjectId>,
    pub board_id: Option<BoardId>,
    pub occurred_at: DateTime<Utc>,
    pub actor: EventActor,
    pub data: DomainEvent,
}

impl EventEnvelope {
    /// Constructs a new envelope, stamping the current time and generating a
    /// fresh UUIDv7 idempotency key.
    pub fn new(
        workspace_id: WorkspaceId,
        project_id: Option<ProjectId>,
        board_id: Option<BoardId>,
        actor: EventActor,
        data: DomainEvent,
    ) -> Self {
        Self {
            id: Uuid::now_v7(),
            event_type: data.event_type().to_string(),
            version: data.event_version(),
            source: EventSource::Internal,
            workspace_id,
            project_id,
            board_id,
            occurred_at: Utc::now(),
            actor,
            data,
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ApiKeyId, UserId};
    use chrono::TimeZone;

    fn nil_uuid() -> Uuid {
        Uuid::nil()
    }

    fn test_envelope(data: DomainEvent) -> EventEnvelope {
        EventEnvelope {
            id: nil_uuid(),
            event_type: data.event_type().to_string(),
            version: data.event_version(),
            source: EventSource::Internal,
            workspace_id: WorkspaceId(nil_uuid()),
            project_id: None,
            board_id: None,
            occurred_at: Utc.with_ymd_and_hms(2026, 6, 30, 12, 0, 0).unwrap(),
            actor: EventActor {
                actor_type: EventActorType::User,
                id: nil_uuid(),
            },
            data,
        }
    }

    fn roundtrip<T>(value: &T)
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
    {
        let json = serde_json::to_string(value).unwrap();
        let back: T = serde_json::from_str(&json).unwrap();
        assert_eq!(value, &back, "round-trip mismatch for {json}");
    }

    // ─── EventSource ─────────────────────────────────────────────────────────

    #[test]
    fn test_source_serializes_to_internal() {
        let json = serde_json::to_value(&EventSource::Internal).unwrap();
        assert_eq!(json, serde_json::Value::String("internal".to_string()));
    }

    #[test]
    fn test_source_roundtrip() {
        roundtrip(&EventSource::Internal);
    }

    #[test]
    fn test_source_forward_compat_external() {
        let src: EventSource = serde_json::from_str(r#""external/github""#).unwrap();
        assert_eq!(src, EventSource::External("external/github".to_string()));
    }

    // ─── DomainEvent serde round-trips (all 14 variants) ─────────────────────

    #[test]
    fn test_task_created_roundtrip() {
        roundtrip(&DomainEvent::TaskCreated(TaskCreatedPayload {
            task_id: TaskId(nil_uuid()),
            title: "My task".to_string(),
            project_id: ProjectId(nil_uuid()),
            board_id: BoardId(nil_uuid()),
            column_id: ColumnId(nil_uuid()),
        }));
    }

    #[test]
    fn test_task_updated_roundtrip() {
        roundtrip(&DomainEvent::TaskUpdated(TaskUpdatedPayload {
            task_id: TaskId(nil_uuid()),
            changed_fields: vec!["title".to_string(), "priority".to_string()],
        }));
    }

    #[test]
    fn test_task_moved_roundtrip() {
        roundtrip(&DomainEvent::TaskMoved(TaskMovedPayload {
            task_id: TaskId(nil_uuid()),
            from_column_id: ColumnId(nil_uuid()),
            to_column_id: ColumnId(nil_uuid()),
        }));
    }

    #[test]
    fn test_task_deleted_roundtrip() {
        roundtrip(&DomainEvent::TaskDeleted(TaskDeletedPayload {
            task_id: TaskId(nil_uuid()),
        }));
    }

    #[test]
    fn test_document_created_roundtrip() {
        roundtrip(&DomainEvent::DocumentCreated(DocumentCreatedPayload {
            document_id: DocumentId(nil_uuid()),
            slug: "my-doc".to_string(),
            title: "My Doc".to_string(),
            project_id: None,
            folder_id: None,
        }));
    }

    #[test]
    fn test_document_updated_roundtrip() {
        roundtrip(&DomainEvent::DocumentUpdated(DocumentUpdatedPayload {
            document_id: DocumentId(nil_uuid()),
            revision_id: RevisionId(nil_uuid()),
            seq: 42,
        }));
    }

    #[test]
    fn test_document_deleted_roundtrip() {
        roundtrip(&DomainEvent::DocumentDeleted(DocumentDeletedPayload {
            document_id: DocumentId(nil_uuid()),
        }));
    }

    #[test]
    fn test_document_moved_roundtrip() {
        roundtrip(&DomainEvent::DocumentMoved(DocumentMovedPayload {
            document_id: DocumentId(nil_uuid()),
            from_folder_id: Some(FolderId(nil_uuid())),
            to_folder_id: None,
            project_id: Some(ProjectId(nil_uuid())),
        }));
    }

    #[test]
    fn test_board_created_roundtrip() {
        roundtrip(&DomainEvent::BoardCreated(BoardCreatedPayload {
            board_id: BoardId(nil_uuid()),
            project_id: ProjectId(nil_uuid()),
            name: "Sprint 1".to_string(),
        }));
    }

    #[test]
    fn test_board_deleted_roundtrip() {
        roundtrip(&DomainEvent::BoardDeleted(BoardDeletedPayload {
            board_id: BoardId(nil_uuid()),
            project_id: ProjectId(nil_uuid()),
        }));
    }

    #[test]
    fn test_column_created_roundtrip() {
        roundtrip(&DomainEvent::ColumnCreated(ColumnCreatedPayload {
            board_id: BoardId(nil_uuid()),
            column_id: ColumnId(nil_uuid()),
            name: "In Progress".to_string(),
        }));
    }

    #[test]
    fn test_column_deleted_roundtrip() {
        roundtrip(&DomainEvent::ColumnDeleted(ColumnDeletedPayload {
            board_id: BoardId(nil_uuid()),
            column_id: ColumnId(nil_uuid()),
        }));
    }

    #[test]
    fn test_folder_created_roundtrip() {
        roundtrip(&DomainEvent::FolderCreated(FolderCreatedPayload {
            folder_id: FolderId(nil_uuid()),
            project_id: ProjectId(nil_uuid()),
            name: "Engineering".to_string(),
        }));
    }

    #[test]
    fn test_folder_deleted_roundtrip() {
        roundtrip(&DomainEvent::FolderDeleted(FolderDeletedPayload {
            folder_id: FolderId(nil_uuid()),
            project_id: ProjectId(nil_uuid()),
        }));
    }

    // ─── Envelope shape and field presence ───────────────────────────────────

    #[test]
    fn test_envelope_required_fields_present() {
        let env = test_envelope(DomainEvent::TaskDeleted(TaskDeletedPayload {
            task_id: TaskId(nil_uuid()),
        }));
        let json: serde_json::Value = serde_json::to_value(&env).unwrap();

        assert!(json["id"].is_string(), "id must be a string");
        assert!(json["version"].is_number(), "version must be a number");
        assert!(json["event_type"].is_string(), "event_type must be a string");
        assert_eq!(json["source"], "internal", "source must equal \"internal\"");
        assert!(json["workspace_id"].is_string(), "workspace_id must be a string");
        assert!(json["occurred_at"].is_string(), "occurred_at must be a string");
    }

    #[test]
    fn test_envelope_event_type_matches_variant() {
        let cases: &[(&str, DomainEvent)] = &[
            ("task.created", DomainEvent::TaskCreated(TaskCreatedPayload {
                task_id: TaskId(nil_uuid()),
                title: "t".to_string(),
                project_id: ProjectId(nil_uuid()),
                board_id: BoardId(nil_uuid()),
                column_id: ColumnId(nil_uuid()),
            })),
            ("task.deleted", DomainEvent::TaskDeleted(TaskDeletedPayload {
                task_id: TaskId(nil_uuid()),
            })),
            ("document.moved", DomainEvent::DocumentMoved(DocumentMovedPayload {
                document_id: DocumentId(nil_uuid()),
                from_folder_id: None,
                to_folder_id: None,
                project_id: None,
            })),
            ("board.created", DomainEvent::BoardCreated(BoardCreatedPayload {
                board_id: BoardId(nil_uuid()),
                project_id: ProjectId(nil_uuid()),
                name: "b".to_string(),
            })),
        ];

        for (expected_type, event) in cases {
            let env = test_envelope(event.clone());
            assert_eq!(
                &env.event_type, expected_type,
                "event_type mismatch for {expected_type}"
            );
            assert_eq!(env.version, 1, "all v1 events must have version=1");
        }
    }

    #[test]
    fn test_envelope_roundtrip_task_created() {
        let env = test_envelope(DomainEvent::TaskCreated(TaskCreatedPayload {
            task_id: TaskId(nil_uuid()),
            title: "hello".to_string(),
            project_id: ProjectId(nil_uuid()),
            board_id: BoardId(nil_uuid()),
            column_id: ColumnId(nil_uuid()),
        }));
        let json = serde_json::to_string(&env).unwrap();
        let back: EventEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(back.event_type, "task.created");
        assert_eq!(back.version, 1);
        assert_eq!(back.source, EventSource::Internal);
    }

    #[test]
    fn test_envelope_roundtrip_document_deleted() {
        let env = test_envelope(DomainEvent::DocumentDeleted(DocumentDeletedPayload {
            document_id: DocumentId(nil_uuid()),
        }));
        let json = serde_json::to_string(&env).unwrap();
        let back: EventEnvelope = serde_json::from_str(&json).unwrap();
        assert!(
            matches!(back.data, DomainEvent::DocumentDeleted(_)),
            "document.deleted must not be misidentified as document.moved"
        );
    }

    // ─── JSON shape snapshots ─────────────────────────────────────────────────

    #[test]
    fn test_task_created_json_shape() {
        let payload = TaskCreatedPayload {
            task_id: TaskId(nil_uuid()),
            title: "hello".to_string(),
            project_id: ProjectId(nil_uuid()),
            board_id: BoardId(nil_uuid()),
            column_id: ColumnId(nil_uuid()),
        };
        let json: serde_json::Value = serde_json::to_value(&payload).unwrap();

        assert!(json["task_id"].is_string());
        assert_eq!(json["title"], "hello");
        assert!(json["project_id"].is_string());
        assert!(json["board_id"].is_string());
        assert!(json["column_id"].is_string());
    }

    #[test]
    fn test_task_updated_json_shape() {
        let payload = TaskUpdatedPayload {
            task_id: TaskId(nil_uuid()),
            changed_fields: vec!["title".to_string()],
        };
        let json: serde_json::Value = serde_json::to_value(&payload).unwrap();

        assert!(json["task_id"].is_string());
        assert!(json["changed_fields"].is_array());
        assert_eq!(json["changed_fields"][0], "title");
    }

    #[test]
    fn test_task_moved_json_shape() {
        let payload = TaskMovedPayload {
            task_id: TaskId(nil_uuid()),
            from_column_id: ColumnId(nil_uuid()),
            to_column_id: ColumnId(nil_uuid()),
        };
        let json: serde_json::Value = serde_json::to_value(&payload).unwrap();

        assert!(json["task_id"].is_string());
        assert!(json["from_column_id"].is_string());
        assert!(json["to_column_id"].is_string());
    }

    #[test]
    fn test_task_deleted_json_shape() {
        let payload = TaskDeletedPayload { task_id: TaskId(nil_uuid()) };
        let json: serde_json::Value = serde_json::to_value(&payload).unwrap();

        assert!(json["task_id"].is_string());
        assert_eq!(json.as_object().unwrap().len(), 1, "task.deleted data must have exactly one key");
    }

    #[test]
    fn test_document_created_json_shape() {
        let payload = DocumentCreatedPayload {
            document_id: DocumentId(nil_uuid()),
            slug: "my-doc".to_string(),
            title: "My Doc".to_string(),
            project_id: Some(ProjectId(nil_uuid())),
            folder_id: None,
        };
        let json: serde_json::Value = serde_json::to_value(&payload).unwrap();

        assert!(json["document_id"].is_string());
        assert_eq!(json["slug"], "my-doc");
        assert_eq!(json["title"], "My Doc");
        assert!(json["project_id"].is_string());
        assert!(json["folder_id"].is_null());
    }

    #[test]
    fn test_document_updated_json_shape() {
        let payload = DocumentUpdatedPayload {
            document_id: DocumentId(nil_uuid()),
            revision_id: RevisionId(nil_uuid()),
            seq: 7,
        };
        let json: serde_json::Value = serde_json::to_value(&payload).unwrap();

        assert!(json["document_id"].is_string());
        assert!(json["revision_id"].is_string());
        assert_eq!(json["seq"], 7);
    }

    #[test]
    fn test_document_deleted_json_shape() {
        let payload = DocumentDeletedPayload { document_id: DocumentId(nil_uuid()) };
        let json: serde_json::Value = serde_json::to_value(&payload).unwrap();

        assert!(json["document_id"].is_string());
        assert_eq!(
            json.as_object().unwrap().len(),
            1,
            "document.deleted data must have exactly one key"
        );
    }

    #[test]
    fn test_document_moved_json_shape() {
        let payload = DocumentMovedPayload {
            document_id: DocumentId(nil_uuid()),
            from_folder_id: Some(FolderId(nil_uuid())),
            to_folder_id: None,
            project_id: Some(ProjectId(nil_uuid())),
        };
        let json: serde_json::Value = serde_json::to_value(&payload).unwrap();

        assert!(json["document_id"].is_string());
        assert!(json["from_folder_id"].is_string());
        assert!(json["to_folder_id"].is_null());
        assert!(json["project_id"].is_string());
    }

    #[test]
    fn test_board_created_json_shape() {
        let payload = BoardCreatedPayload {
            board_id: BoardId(nil_uuid()),
            project_id: ProjectId(nil_uuid()),
            name: "Sprint".to_string(),
        };
        let json: serde_json::Value = serde_json::to_value(&payload).unwrap();

        assert!(json["board_id"].is_string());
        assert!(json["project_id"].is_string());
        assert_eq!(json["name"], "Sprint");
    }

    #[test]
    fn test_board_deleted_json_shape() {
        let payload = BoardDeletedPayload {
            board_id: BoardId(nil_uuid()),
            project_id: ProjectId(nil_uuid()),
        };
        let json: serde_json::Value = serde_json::to_value(&payload).unwrap();

        assert!(json["board_id"].is_string());
        assert!(json["project_id"].is_string());
    }

    #[test]
    fn test_column_created_json_shape() {
        let payload = ColumnCreatedPayload {
            board_id: BoardId(nil_uuid()),
            column_id: ColumnId(nil_uuid()),
            name: "Done".to_string(),
        };
        let json: serde_json::Value = serde_json::to_value(&payload).unwrap();

        assert!(json["board_id"].is_string());
        assert!(json["column_id"].is_string());
        assert_eq!(json["name"], "Done");
    }

    #[test]
    fn test_column_deleted_json_shape() {
        let payload = ColumnDeletedPayload {
            board_id: BoardId(nil_uuid()),
            column_id: ColumnId(nil_uuid()),
        };
        let json: serde_json::Value = serde_json::to_value(&payload).unwrap();

        assert!(json["board_id"].is_string());
        assert!(json["column_id"].is_string());
    }

    #[test]
    fn test_folder_created_json_shape() {
        let payload = FolderCreatedPayload {
            folder_id: FolderId(nil_uuid()),
            project_id: ProjectId(nil_uuid()),
            name: "Docs".to_string(),
        };
        let json: serde_json::Value = serde_json::to_value(&payload).unwrap();

        assert!(json["folder_id"].is_string());
        assert!(json["project_id"].is_string());
        assert_eq!(json["name"], "Docs");
    }

    #[test]
    fn test_folder_deleted_json_shape() {
        let payload = FolderDeletedPayload {
            folder_id: FolderId(nil_uuid()),
            project_id: ProjectId(nil_uuid()),
        };
        let json: serde_json::Value = serde_json::to_value(&payload).unwrap();

        assert!(json["folder_id"].is_string());
        assert!(json["project_id"].is_string());
    }

    // ─── Version constants ────────────────────────────────────────────────────

    #[test]
    fn test_all_version_consts_are_one() {
        use version::*;
        assert_eq!(TASK_CREATED, 1);
        assert_eq!(TASK_UPDATED, 1);
        assert_eq!(TASK_MOVED, 1);
        assert_eq!(TASK_DELETED, 1);
        assert_eq!(DOCUMENT_CREATED, 1);
        assert_eq!(DOCUMENT_UPDATED, 1);
        assert_eq!(DOCUMENT_MOVED, 1);
        assert_eq!(DOCUMENT_DELETED, 1);
        assert_eq!(BOARD_CREATED, 1);
        assert_eq!(BOARD_DELETED, 1);
        assert_eq!(COLUMN_CREATED, 1);
        assert_eq!(COLUMN_DELETED, 1);
        assert_eq!(FOLDER_CREATED, 1);
        assert_eq!(FOLDER_DELETED, 1);
    }

    // ─── EventActor ──────────────────────────────────────────────────────────

    #[test]
    fn test_event_actor_user_shape() {
        let actor = EventActor {
            actor_type: EventActorType::User,
            id: nil_uuid(),
        };
        let json: serde_json::Value = serde_json::to_value(&actor).unwrap();

        assert_eq!(json["type"], "user");
        assert!(json["id"].is_string());
    }

    #[test]
    fn test_event_actor_api_key_shape() {
        let actor = EventActor {
            actor_type: EventActorType::ApiKey,
            id: nil_uuid(),
        };
        let json: serde_json::Value = serde_json::to_value(&actor).unwrap();

        assert_eq!(json["type"], "api_key");
    }

    #[test]
    fn test_from_actor_user() {
        let uid = UserId(nil_uuid());
        let ea = EventActor::from(Actor::User(uid));
        assert_eq!(ea.actor_type, EventActorType::User);
        assert_eq!(ea.id, uid.0);
    }

    #[test]
    fn test_from_actor_api_key() {
        let kid = ApiKeyId(nil_uuid());
        let ea = EventActor::from(Actor::ApiKey(kid));
        assert_eq!(ea.actor_type, EventActorType::ApiKey);
        assert_eq!(ea.id, kid.0);
    }
}
