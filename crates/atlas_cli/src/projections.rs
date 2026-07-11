#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use atlas_api::dtos::audit::AuditEntryDto;
use atlas_api::dtos::boards_tasks::{
    ActivityEntryDto, AssigneeDto, BoardSummaryDto, ChecklistItemDto, ColumnDto, CommentDto,
    PromotionDto, ReferenceDto, TaskAttachmentDto, TaskBacklinkDto, TaskDto, TaskSummaryDto,
    UnifiedReferenceDto,
};
use atlas_api::dtos::documents::{
    ActorDto, AttachmentDto, BacklinkDto, DocumentDto, DocumentSummaryDto, RevisionContentDto,
    RevisionMetaDto,
};
use atlas_api::dtos::folders::FolderDto;
use atlas_api::dtos::groups::{GroupDto, GroupMemberDto};
use atlas_api::dtos::property_definitions::PropertyDefinitionDto;
use atlas_api::dtos::saved_searches::SavedSearchDto;
use atlas_api::dtos::search::{SearchHitDto, SearchKindDto};
use atlas_api::dtos::status_templates::StatusTemplateDto;
use atlas_api::dtos::tags::TagDto;
use atlas_api::dtos::task_views::{TaskViewDto, TaskViewFiltersDto};
use atlas_api::dtos::{
    ActivationLinkResponse, ApiKeyCreated, ApiKeyDto, ApiKeyGrantDto, ApiKeyScope,
    CreateUserResponse, GrantDto, GrantPrincipal, GrantedByDto, PrincipalDto, ProjectDto, UserDto,
    UserMembershipDto, WorkspaceDto,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use uuid::Uuid;

use crate::output::TableRow;

/// Standard pagination envelope used for all list responses.
#[derive(Debug, Serialize)]
pub(crate) struct Envelope<T: Serialize> {
    pub(crate) items: Vec<T>,
    pub(crate) next_cursor: Option<String>,
    pub(crate) has_more: bool,
}

/// CLI projection for a search hit, mirroring the MCP search-hit shape exactly.
///
/// Optional fields use `skip_serializing_if` so they are omitted (not `null`)
/// when absent, matching the MCP wire format.
#[derive(Debug, Serialize)]
pub(crate) struct SearchHitProjection {
    pub(crate) id: Uuid,
    pub(crate) kind: String,
    pub(crate) title: String,
    pub(crate) score: f32,
    pub(crate) updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) readable_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) snippet: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) project_slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) column_name: Option<String>,
}

impl From<SearchHitDto> for SearchHitProjection {
    fn from(dto: SearchHitDto) -> Self {
        let kind = match dto.kind {
            SearchKindDto::Document => "document".to_owned(),
            SearchKindDto::Task => "task".to_owned(),
        };

        Self {
            id: dto.id,
            kind,
            title: dto.title,
            score: dto.score,
            updated_at: dto.updated_at,
            readable_id: dto.readable_id,
            snippet: dto.snippet,
            project_slug: dto.project_slug,
            column_name: dto.column_name,
        }
    }
}

impl TableRow for SearchHitProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Kind", "Title", "Score", "Updated"]
    }

    fn row(&self) -> Vec<String> {
        let id_display = self
            .readable_id
            .as_deref()
            .map(|r| format!("{} [{}]", self.id, r))
            .unwrap_or_else(|| self.id.to_string());

        vec![
            id_display,
            self.kind.clone(),
            self.title.clone(),
            format!("{:.4}", self.score),
            self.updated_at.format("%Y-%m-%d").to_string(),
        ]
    }
}

// ---------------------------------------------------------------------------
// Task projections
// ---------------------------------------------------------------------------

/// Compact assignee shape used in task summary rows.
///
/// Mirrors the `{type, display_name}` pair emitted by the MCP `list_tasks`
/// response. `type` is serialized as the JSON key; stored as `type_` to avoid
/// conflict with the Rust keyword.
#[derive(Debug, Serialize)]
pub(crate) struct AssigneeProjection {
    #[serde(rename = "type")]
    pub(crate) type_: String,
    pub(crate) display_name: Option<String>,
}

impl From<ActorDto> for AssigneeProjection {
    fn from(a: ActorDto) -> Self {
        Self {
            type_: a.r#type,
            display_name: a.display_name,
        }
    }
}

impl TableRow for AssigneeProjection {
    fn headers() -> &'static [&'static str] {
        &["Type", "Display Name"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.type_.clone(),
            self.display_name.clone().unwrap_or_default(),
        ]
    }
}

/// Task list row projection (from `TaskSummaryDto`).
#[derive(Debug, Serialize)]
pub(crate) struct TaskSummaryProjection {
    pub(crate) readable_id: String,
    pub(crate) title: String,
    pub(crate) board_name: String,
    pub(crate) column_name: String,
    pub(crate) priority: Option<String>,
    pub(crate) labels: Vec<String>,
    pub(crate) estimate: Option<i32>,
    pub(crate) assignees: Vec<AssigneeProjection>,
    pub(crate) updated_at: DateTime<Utc>,
}

impl From<TaskSummaryDto> for TaskSummaryProjection {
    fn from(dto: TaskSummaryDto) -> Self {
        let assignees = dto
            .assignees
            .into_iter()
            .map(AssigneeProjection::from)
            .collect();

        Self {
            readable_id: dto.readable_id,
            title: dto.title,
            board_name: dto.board_name,
            column_name: dto.column_name,
            priority: dto.priority,
            labels: dto.labels,
            estimate: dto.estimate,
            assignees,
            updated_at: dto.updated_at,
        }
    }
}

impl TableRow for TaskSummaryProjection {
    fn headers() -> &'static [&'static str] {
        &[
            "ID", "Title", "Board", "Column", "Priority", "Labels", "Est.", "Updated",
        ]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.readable_id.clone(),
            self.title.clone(),
            self.board_name.clone(),
            self.column_name.clone(),
            self.priority.clone().unwrap_or_default(),
            self.labels.join(", "),
            self.estimate.map(|e| e.to_string()).unwrap_or_default(),
            self.updated_at.format("%Y-%m-%d").to_string(),
        ]
    }
}

/// Compact task projection (from `TaskDto`).
///
/// `board_name` and `column_name` are omitted when the server returns an empty
/// string — mutation responses do not populate them, so an empty value would be
/// misleading rather than informative.
#[derive(Debug, Serialize)]
pub(crate) struct TaskCompactProjection {
    pub(crate) readable_id: String,
    pub(crate) title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) board_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) column_name: Option<String>,
    pub(crate) priority: Option<String>,
    pub(crate) labels: Vec<String>,
    pub(crate) estimate: Option<i32>,
    pub(crate) due_date: Option<DateTime<Utc>>,
    pub(crate) parent_task_id: Option<Uuid>,
    pub(crate) updated_at: DateTime<Utc>,
}

impl From<TaskDto> for TaskCompactProjection {
    fn from(task: TaskDto) -> Self {
        let board_name = if task.board_name.is_empty() {
            None
        } else {
            Some(task.board_name)
        };

        let column_name = if task.column_name.is_empty() {
            None
        } else {
            Some(task.column_name)
        };

        Self {
            readable_id: task.readable_id,
            title: task.title,
            board_name,
            column_name,
            priority: task.priority,
            labels: task.labels,
            estimate: task.estimate,
            due_date: task.due_date,
            parent_task_id: task.parent_task_id,
            updated_at: task.updated_at,
        }
    }
}

impl TableRow for TaskCompactProjection {
    fn headers() -> &'static [&'static str] {
        &[
            "ID", "Title", "Board", "Column", "Priority", "Labels", "Est.", "Due", "Updated",
        ]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.readable_id.clone(),
            self.title.clone(),
            self.board_name.as_deref().unwrap_or("").to_owned(),
            self.column_name.as_deref().unwrap_or("").to_owned(),
            self.priority.clone().unwrap_or_default(),
            self.labels.join(", "),
            self.estimate.map(|e| e.to_string()).unwrap_or_default(),
            self.due_date
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_default(),
            self.updated_at.format("%Y-%m-%d").to_string(),
        ]
    }
}

/// Full task projection (`TaskDto` plus all sub-resources).
///
/// Sub-resources (references, subtasks, assignees) each have an error fallback:
/// when a backing client call fails, `*_error` is set and the value field is
/// absent. Exactly one of each pair is `Some` at runtime; `skip_serializing_if`
/// ensures the absent member is omitted from the JSON output.
#[derive(Debug, Serialize)]
pub(crate) struct TaskFullProjection {
    pub(crate) readable_id: String,
    pub(crate) title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) board_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) column_name: Option<String>,
    pub(crate) priority: Option<String>,
    pub(crate) labels: Vec<String>,
    pub(crate) estimate: Option<i32>,
    pub(crate) due_date: Option<DateTime<Utc>>,
    pub(crate) parent_task_id: Option<Uuid>,
    pub(crate) updated_at: DateTime<Utc>,
    pub(crate) description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) references: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) references_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) subtasks: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) subtasks_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) assignees: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) assignees_error: Option<String>,
}

impl TaskFullProjection {
    /// Assembles the full task projection from a task and its sub-resource results.
    ///
    /// Each sub-resource is provided as a `Result`: `Ok(items)` populates the
    /// value field; `Err(msg)` populates the corresponding `*_error` field instead.
    pub(crate) fn new(
        task: TaskDto,
        refs: Result<Vec<serde_json::Value>, String>,
        subtasks: Result<Vec<serde_json::Value>, String>,
        assignees: Result<Vec<serde_json::Value>, String>,
    ) -> Self {
        let board_name = if task.board_name.is_empty() {
            None
        } else {
            Some(task.board_name)
        };

        let column_name = if task.column_name.is_empty() {
            None
        } else {
            Some(task.column_name)
        };

        let (references, references_error) = match refs {
            Ok(v) => (Some(v), None),
            Err(e) => (None, Some(e)),
        };

        let (subtasks, subtasks_error) = match subtasks {
            Ok(v) => (Some(v), None),
            Err(e) => (None, Some(e)),
        };

        let (assignees, assignees_error) = match assignees {
            Ok(v) => (Some(v), None),
            Err(e) => (None, Some(e)),
        };

        Self {
            readable_id: task.readable_id,
            title: task.title,
            board_name,
            column_name,
            priority: task.priority,
            labels: task.labels,
            estimate: task.estimate,
            due_date: task.due_date,
            parent_task_id: task.parent_task_id,
            updated_at: task.updated_at,
            description: task.description,
            references,
            references_error,
            subtasks,
            subtasks_error,
            assignees,
            assignees_error,
        }
    }
}

impl TableRow for TaskFullProjection {
    fn headers() -> &'static [&'static str] {
        &[
            "ID", "Title", "Board", "Column", "Priority", "Labels", "Est.", "Due", "Updated",
        ]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.readable_id.clone(),
            self.title.clone(),
            self.board_name.as_deref().unwrap_or("").to_owned(),
            self.column_name.as_deref().unwrap_or("").to_owned(),
            self.priority.clone().unwrap_or_default(),
            self.labels.join(", "),
            self.estimate.map(|e| e.to_string()).unwrap_or_default(),
            self.due_date
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_default(),
            self.updated_at.format("%Y-%m-%d").to_string(),
        ]
    }
}

/// Projection for a completed task deletion.
///
/// Uses `readable_id` (not a generic `id`) to match the resource's public
/// identifier, consistent with how tasks are addressed across the CLI.
#[derive(Debug, Serialize)]
pub(crate) struct DeleteTaskProjection {
    pub(crate) deleted: bool,
    pub(crate) readable_id: String,
}

impl TableRow for DeleteTaskProjection {
    fn headers() -> &'static [&'static str] {
        &["Deleted", "ID"]
    }

    fn row(&self) -> Vec<String> {
        vec![self.deleted.to_string(), self.readable_id.clone()]
    }
}

// ---------------------------------------------------------------------------
// Document projections
// ---------------------------------------------------------------------------

/// Summary document projection used for `docs list` responses.
///
/// Matches the `project_document_summary` shape from the MCP: optional fields
/// (`slug`, `folder_id`) are omitted when absent rather than serialized as null.
#[derive(Debug, Serialize)]
pub(crate) struct DocSummaryProjection {
    pub(crate) id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) slug: Option<String>,
    pub(crate) title: String,
    pub(crate) head_seq: i64,
    pub(crate) updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) folder_id: Option<Uuid>,
}

impl From<DocumentSummaryDto> for DocSummaryProjection {
    fn from(doc: DocumentSummaryDto) -> Self {
        Self {
            id: doc.id,
            slug: doc.slug,
            title: doc.title,
            head_seq: doc.head_seq,
            updated_at: doc.updated_at,
            folder_id: doc.folder_id,
        }
    }
}

impl TableRow for DocSummaryProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Slug", "Title", "Seq", "Updated"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.id.to_string(),
            self.slug.clone().unwrap_or_default(),
            self.title.clone(),
            self.head_seq.to_string(),
            self.updated_at.format("%Y-%m-%d").to_string(),
        ]
    }
}

/// Compact document projection mirroring `project_document_compact` exactly.
///
/// All eight fields are always present (optional values serialize as `null`
/// when absent), matching the MCP wire format.
#[derive(Debug, Serialize)]
pub(crate) struct DocCompactProjection {
    pub(crate) id: Uuid,
    pub(crate) slug: Option<String>,
    pub(crate) title: String,
    pub(crate) head_revision_id: Uuid,
    pub(crate) head_seq: i64,
    pub(crate) updated_at: DateTime<Utc>,
    pub(crate) folder_id: Option<Uuid>,
    pub(crate) project_id: Option<Uuid>,
}

impl From<DocumentDto> for DocCompactProjection {
    fn from(doc: DocumentDto) -> Self {
        Self {
            id: doc.id,
            slug: doc.slug,
            title: doc.title,
            head_revision_id: doc.head_revision_id,
            head_seq: doc.head_seq,
            updated_at: doc.updated_at,
            folder_id: doc.folder_id,
            project_id: doc.project_id,
        }
    }
}

impl TableRow for DocCompactProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Slug", "Title", "Rev", "Seq", "Updated"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.id.to_string(),
            self.slug.clone().unwrap_or_default(),
            self.title.clone(),
            self.head_revision_id.to_string(),
            self.head_seq.to_string(),
            self.updated_at.format("%Y-%m-%d").to_string(),
        ]
    }
}

/// Full document projection mirroring `project_document_full`: compact fields
/// plus markdown content and frontmatter.
#[derive(Debug, Serialize)]
pub(crate) struct DocFullProjection {
    pub(crate) id: Uuid,
    pub(crate) slug: Option<String>,
    pub(crate) title: String,
    pub(crate) head_revision_id: Uuid,
    pub(crate) head_seq: i64,
    pub(crate) updated_at: DateTime<Utc>,
    pub(crate) folder_id: Option<Uuid>,
    pub(crate) project_id: Option<Uuid>,
    pub(crate) content: String,
    pub(crate) frontmatter: serde_json::Value,
}

impl From<DocumentDto> for DocFullProjection {
    fn from(doc: DocumentDto) -> Self {
        Self {
            id: doc.id,
            slug: doc.slug,
            title: doc.title,
            head_revision_id: doc.head_revision_id,
            head_seq: doc.head_seq,
            updated_at: doc.updated_at,
            folder_id: doc.folder_id,
            project_id: doc.project_id,
            content: doc.content,
            frontmatter: doc.frontmatter,
        }
    }
}

impl TableRow for DocFullProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Slug", "Title", "Rev", "Seq", "Updated"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.id.to_string(),
            self.slug.clone().unwrap_or_default(),
            self.title.clone(),
            self.head_revision_id.to_string(),
            self.head_seq.to_string(),
            self.updated_at.format("%Y-%m-%d").to_string(),
        ]
    }
}

/// Projection for a completed document deletion.
///
/// Uses `slug` (not a generic `id`) to match the document's public identifier,
/// consistent with how documents are addressed across the CLI and the MCP.
#[derive(Debug, Serialize)]
pub(crate) struct DeleteDocProjection {
    pub(crate) deleted: bool,
    pub(crate) slug: String,
}

impl TableRow for DeleteDocProjection {
    fn headers() -> &'static [&'static str] {
        &["Deleted", "Slug"]
    }

    fn row(&self) -> Vec<String> {
        vec![self.deleted.to_string(), self.slug.clone()]
    }
}

// ---------------------------------------------------------------------------
// Structure projections (Batch 3)
// ---------------------------------------------------------------------------

/// Workspace projection mirroring `project_workspace` from the MCP.
///
/// `created_at` is dropped; the slug is the primary reference for subsequent
/// scoped calls.
#[derive(Debug, Serialize)]
pub(crate) struct WorkspaceProjection {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    pub(crate) slug: String,
    pub(crate) updated_at: DateTime<Utc>,
}

impl From<WorkspaceDto> for WorkspaceProjection {
    fn from(ws: WorkspaceDto) -> Self {
        Self {
            id: ws.id,
            name: ws.name,
            slug: ws.slug,
            updated_at: ws.updated_at,
        }
    }
}

impl TableRow for WorkspaceProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Name", "Slug", "Updated"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.id.to_string(),
            self.name.clone(),
            self.slug.clone(),
            self.updated_at.format("%Y-%m-%d").to_string(),
        ]
    }
}

/// Project projection mirroring `project_project` from the MCP.
///
/// `workspace_id` and `created_at` are dropped. `visibility_role` is omitted
/// when absent (only present on non-public projects with an explicit grant role).
#[derive(Debug, Serialize)]
pub(crate) struct ProjectProjection {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    pub(crate) slug: String,
    pub(crate) task_prefix: String,
    pub(crate) visibility: String,
    pub(crate) updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) visibility_role: Option<String>,
}

impl From<ProjectDto> for ProjectProjection {
    fn from(p: ProjectDto) -> Self {
        Self {
            id: p.id,
            name: p.name,
            slug: p.slug,
            task_prefix: p.task_prefix,
            visibility: p.visibility,
            updated_at: p.updated_at,
            visibility_role: p.visibility_role,
        }
    }
}

impl TableRow for ProjectProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Name", "Slug", "Prefix", "Visibility", "Updated"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.id.to_string(),
            self.name.clone(),
            self.slug.clone(),
            self.task_prefix.clone(),
            self.visibility.clone(),
            self.updated_at.format("%Y-%m-%d").to_string(),
        ]
    }
}

/// Board list-row projection mirroring `project_board_summary` from the MCP.
///
/// `created_at` is dropped; `id` and `name` are the primary references.
#[derive(Debug, Serialize)]
pub(crate) struct BoardProjection {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    pub(crate) updated_at: DateTime<Utc>,
}

impl From<BoardSummaryDto> for BoardProjection {
    fn from(b: BoardSummaryDto) -> Self {
        Self {
            id: b.id,
            name: b.name,
            updated_at: b.updated_at,
        }
    }
}

impl TableRow for BoardProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Name", "Updated"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.id.to_string(),
            self.name.clone(),
            self.updated_at.format("%Y-%m-%d").to_string(),
        ]
    }
}

/// Column projection mirroring `project_column` from the MCP.
///
/// `board_id`, `position_key`, and timestamps are dropped. `color` is omitted
/// when absent.
#[derive(Debug, Serialize)]
pub(crate) struct ColumnProjection {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) color: Option<String>,
}

impl From<ColumnDto> for ColumnProjection {
    fn from(col: ColumnDto) -> Self {
        Self {
            id: col.id,
            name: col.name,
            color: col.color,
        }
    }
}

impl TableRow for ColumnProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Name", "Color"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.id.to_string(),
            self.name.clone(),
            self.color.clone().unwrap_or_default(),
        ]
    }
}

/// Tag projection mirroring `project_tag` from the MCP.
///
/// `workspace_id` and timestamps are dropped. `color` is omitted when absent.
#[derive(Debug, Serialize)]
pub(crate) struct TagProjection {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) color: Option<String>,
}

impl From<TagDto> for TagProjection {
    fn from(tag: TagDto) -> Self {
        Self {
            id: tag.id,
            name: tag.name,
            color: tag.color,
        }
    }
}

impl TableRow for TagProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Name", "Color"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.id.to_string(),
            self.name.clone(),
            self.color.clone().unwrap_or_default(),
        ]
    }
}

/// Member/principal projection mirroring `project_principal` from the MCP.
///
/// Exposes `principal_type`, `id`, and `display` — the minimum needed to
/// resolve a human name to the id format required by assignee filters.
#[derive(Debug, Serialize)]
pub(crate) struct MemberProjection {
    pub(crate) principal_type: String,
    pub(crate) id: Uuid,
    pub(crate) display: String,
}

impl From<PrincipalDto> for MemberProjection {
    fn from(p: PrincipalDto) -> Self {
        Self {
            principal_type: p.principal_type,
            id: p.id,
            display: p.display,
        }
    }
}

impl TableRow for MemberProjection {
    fn headers() -> &'static [&'static str] {
        &["Type", "ID", "Display"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.principal_type.clone(),
            self.id.to_string(),
            self.display.clone(),
        ]
    }
}

/// Folder projection mirroring `project_folder` from the MCP.
///
/// `workspace_id`, `project_id`, and `created_at` are dropped.
/// `parent_folder_id` is omitted when absent.
#[derive(Debug, Serialize)]
pub(crate) struct FolderProjection {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    pub(crate) updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) parent_folder_id: Option<Uuid>,
}

impl From<FolderDto> for FolderProjection {
    fn from(f: FolderDto) -> Self {
        Self {
            id: f.id,
            name: f.name,
            updated_at: f.updated_at,
            parent_folder_id: f.parent_folder_id,
        }
    }
}

impl TableRow for FolderProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Name", "Updated", "Parent"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.id.to_string(),
            self.name.clone(),
            self.updated_at.format("%Y-%m-%d").to_string(),
            self.parent_folder_id
                .map(|id| id.to_string())
                .unwrap_or_default(),
        ]
    }
}

// ---------------------------------------------------------------------------
// Graph / activity projections (Batch 4)
// ---------------------------------------------------------------------------

/// Outbound reference on a task, mirroring `project_reference` from the MCP.
///
/// `id` (UUID) and attribution fields are dropped; `kind` + target fields identify
/// the reference. Optional target fields are omitted when absent.
#[derive(Debug, Serialize)]
pub(crate) struct TaskRefProjection {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) manual_reference_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) manual_kind: Option<String>,
    pub(crate) origins: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) target_readable_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) target_document_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) target_title: Option<String>,
    pub(crate) target_resolved: bool,
}

impl From<UnifiedReferenceDto> for TaskRefProjection {
    fn from(r: UnifiedReferenceDto) -> Self {
        Self {
            manual_reference_id: r.manual_reference_id,
            manual_kind: r.manual_kind,
            origins: r
                .origins
                .into_iter()
                .map(|origin| match origin {
                    atlas_api::dtos::boards_tasks::ReferenceOriginDto::Manual => "manual".into(),
                    atlas_api::dtos::boards_tasks::ReferenceOriginDto::Wikilink => {
                        "wikilink".into()
                    }
                })
                .collect(),
            target_readable_id: r.target_readable_id,
            target_document_id: r.target_document_id,
            target_title: r.target_title,
            target_resolved: r.target_resolved,
        }
    }
}

impl From<ReferenceDto> for TaskRefProjection {
    fn from(r: ReferenceDto) -> Self {
        Self {
            manual_reference_id: Some(r.id),
            manual_kind: Some(r.kind),
            origins: vec!["manual".into()],
            target_readable_id: r.target_readable_id,
            target_document_id: r.target_document_id,
            target_title: r.target_title,
            target_resolved: r.target_resolved,
        }
    }
}

impl TableRow for TaskRefProjection {
    fn headers() -> &'static [&'static str] {
        &[
            "Manual Reference ID",
            "Manual Kind",
            "Origins",
            "Target Task",
            "Target Doc",
            "Title",
            "Resolved",
        ]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.manual_reference_id
                .map(|id| id.to_string())
                .unwrap_or_default(),
            self.manual_kind.clone().unwrap_or_default(),
            self.origins.join(","),
            self.target_readable_id.clone().unwrap_or_default(),
            self.target_document_id
                .map(|id| id.to_string())
                .unwrap_or_default(),
            self.target_title.clone().unwrap_or_default(),
            self.target_resolved.to_string(),
        ]
    }
}

/// Inbound reference (backlink) on a task, mirroring `project_task_backlink` from the MCP.
///
/// `source_task_id` (UUID) is dropped; `source_readable_id` is the public handle.
#[derive(Debug, Serialize)]
pub(crate) struct TaskBacklinkProjection {
    pub(crate) source_readable_id: String,
    pub(crate) source_title: String,
    pub(crate) kind: String,
}

impl From<TaskBacklinkDto> for TaskBacklinkProjection {
    fn from(b: TaskBacklinkDto) -> Self {
        Self {
            source_readable_id: b.source_readable_id,
            source_title: b.source_title,
            kind: b.kind,
        }
    }
}

impl TableRow for TaskBacklinkProjection {
    fn headers() -> &'static [&'static str] {
        &["Source ID", "Source Title", "Kind"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.source_readable_id.clone(),
            self.source_title.clone(),
            self.kind.clone(),
        ]
    }
}

/// Full assignee entry on a task, mirroring `project_assignee` from the MCP.
///
/// Exposes the actor's `type` and `display_name` from the nested `assignee` sub-DTO,
/// plus `assigned_at`. `assigned_by` is dropped.
#[derive(Debug, Serialize)]
pub(crate) struct TaskAssigneeProjection {
    #[serde(rename = "type")]
    pub(crate) type_: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) display_name: Option<String>,
    pub(crate) assigned_at: DateTime<Utc>,
}

impl From<AssigneeDto> for TaskAssigneeProjection {
    fn from(a: AssigneeDto) -> Self {
        Self {
            type_: a.assignee.r#type,
            display_name: a.assignee.display_name,
            assigned_at: a.assigned_at,
        }
    }
}

impl TableRow for TaskAssigneeProjection {
    fn headers() -> &'static [&'static str] {
        &["Type", "Display Name", "Assigned At"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.type_.clone(),
            self.display_name.clone().unwrap_or_default(),
            self.assigned_at.format("%Y-%m-%d").to_string(),
        ]
    }
}

/// Checklist item projection, mirroring `project_checklist_item` from the MCP.
///
/// `task_id`, `position_key`, and timestamps are dropped. `promoted_readable_id`
/// is preserved so callers can navigate to the promoted task.
#[derive(Debug, Serialize)]
pub(crate) struct ChecklistItemProjection {
    pub(crate) id: Uuid,
    pub(crate) title: String,
    pub(crate) checked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) promoted_readable_id: Option<String>,
}

impl From<ChecklistItemDto> for ChecklistItemProjection {
    fn from(item: ChecklistItemDto) -> Self {
        Self {
            id: item.id,
            title: item.title,
            checked: item.checked,
            promoted_readable_id: item.promoted_readable_id,
        }
    }
}

impl TableRow for ChecklistItemProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Title", "Checked", "Promoted To"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.id.to_string(),
            self.title.clone(),
            self.checked.to_string(),
            self.promoted_readable_id.clone().unwrap_or_default(),
        ]
    }
}

/// Result of promoting a checklist item to a task, mirroring `project_promotion` from the MCP.
///
/// Surfaces the new task's compact fields and the original checklist item.
/// `parent_reference` is omitted when absent (the promoted task has no parent ref).
#[derive(Debug, Serialize)]
pub(crate) struct PromotionProjection {
    pub(crate) task: TaskCompactProjection,
    pub(crate) checklist_item: ChecklistItemProjection,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) parent_reference: Option<TaskRefProjection>,
}

impl From<PromotionDto> for PromotionProjection {
    fn from(p: PromotionDto) -> Self {
        Self {
            task: TaskCompactProjection::from(p.task),
            checklist_item: ChecklistItemProjection::from(p.checklist_item),
            parent_reference: p.parent_reference.map(TaskRefProjection::from),
        }
    }
}

impl TableRow for PromotionProjection {
    fn headers() -> &'static [&'static str] {
        &["New Task ID", "New Task Title", "Checklist Item", "Checked"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.task.readable_id.clone(),
            self.task.title.clone(),
            self.checklist_item.title.clone(),
            self.checklist_item.checked.to_string(),
        ]
    }
}

/// Task-scoped activity entry, mirroring `project_activity_entry` from the MCP.
///
/// `id` (UUID) and `task_id` are dropped. `payload` is verbatim because its schema
/// varies per `kind`.
#[derive(Debug, Serialize)]
pub(crate) struct TaskActivityProjection {
    pub(crate) kind: String,
    pub(crate) actor: AssigneeProjection,
    pub(crate) payload: serde_json::Value,
    pub(crate) created_at: DateTime<Utc>,
}

impl From<ActivityEntryDto> for TaskActivityProjection {
    fn from(entry: ActivityEntryDto) -> Self {
        Self {
            kind: entry.kind,
            actor: AssigneeProjection::from(entry.actor),
            payload: entry.payload,
            created_at: entry.created_at,
        }
    }
}

impl TableRow for TaskActivityProjection {
    fn headers() -> &'static [&'static str] {
        &["Kind", "Actor", "Created At"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.kind.clone(),
            self.actor
                .display_name
                .clone()
                .unwrap_or_else(|| self.actor.type_.clone()),
            self.created_at.format("%Y-%m-%d %H:%M").to_string(),
        ]
    }
}

/// Outbound comment on a task, mirroring `project_comment` from the MCP.
///
/// `task_id` / `document_id` are dropped — the owner is implicit from the command
/// the comment was listed under. `updated_at` is surfaced so an edited comment is
/// distinguishable from an untouched one.
#[derive(Debug, Serialize)]
pub(crate) struct CommentProjection {
    pub(crate) id: Uuid,
    pub(crate) author: AssigneeProjection,
    pub(crate) body: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

impl From<CommentDto> for CommentProjection {
    fn from(c: CommentDto) -> Self {
        Self {
            id: c.id,
            author: AssigneeProjection::from(c.author),
            body: c.body,
            created_at: c.created_at,
            updated_at: c.updated_at,
        }
    }
}

impl TableRow for CommentProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Author", "Body", "Created At"]
    }

    fn row(&self) -> Vec<String> {
        let body = if self.body.chars().count() > 60 {
            format!("{}…", self.body.chars().take(59).collect::<String>())
        } else {
            self.body.clone()
        };

        vec![
            self.id.to_string(),
            self.author
                .display_name
                .clone()
                .unwrap_or_else(|| self.author.type_.clone()),
            body.replace('\n', " "),
            self.created_at.format("%Y-%m-%d %H:%M").to_string(),
        ]
    }
}

/// Subtask projection, aliasing `TaskSummaryProjection` for semantic clarity.
///
/// Subtasks are fetched as `TaskSummaryDto` and share the exact field set of a
/// task list row, including `assignees`. This alias lets handlers call
/// `SubtaskProjection::from(dto)` without duplicating the struct definition.
pub(crate) type SubtaskProjection = TaskSummaryProjection;

/// Document history entry (revision metadata), mirroring `project_revision_meta` from the MCP.
///
/// `id` (UUID) is dropped; `seq` is the stable handle for fetching full revision content.
/// `actor` is absent when the revision was created by a system operation.
#[derive(Debug, Serialize)]
pub(crate) struct DocHistoryProjection {
    pub(crate) seq: i64,
    pub(crate) is_anchor: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) actor: Option<AssigneeProjection>,
    pub(crate) created_at: DateTime<Utc>,
}

impl From<RevisionMetaDto> for DocHistoryProjection {
    fn from(rev: RevisionMetaDto) -> Self {
        Self {
            seq: rev.seq,
            is_anchor: rev.is_anchor,
            actor: rev.actor.map(AssigneeProjection::from),
            created_at: rev.created_at,
        }
    }
}

impl TableRow for DocHistoryProjection {
    fn headers() -> &'static [&'static str] {
        &["Seq", "Anchor", "Actor", "Created At"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.seq.to_string(),
            self.is_anchor.to_string(),
            self.actor
                .as_ref()
                .and_then(|a| a.display_name.clone())
                .unwrap_or_default(),
            self.created_at.format("%Y-%m-%d %H:%M").to_string(),
        ]
    }
}

/// Full document revision content, mirroring `project_revision_content` from the MCP.
///
/// `id` (UUID) is dropped; `seq` + `content` are the load-bearing fields.
/// `actor` is absent when no attribution is available.
#[derive(Debug, Serialize)]
pub(crate) struct DocRevisionProjection {
    pub(crate) seq: i64,
    pub(crate) content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) actor: Option<AssigneeProjection>,
    pub(crate) created_at: DateTime<Utc>,
}

impl From<RevisionContentDto> for DocRevisionProjection {
    fn from(rev: RevisionContentDto) -> Self {
        Self {
            seq: rev.seq,
            content: rev.content,
            actor: rev.actor.map(AssigneeProjection::from),
            created_at: rev.created_at,
        }
    }
}

impl TableRow for DocRevisionProjection {
    fn headers() -> &'static [&'static str] {
        &["Seq", "Actor", "Created At", "Content (preview)"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.seq.to_string(),
            self.actor
                .as_ref()
                .and_then(|a| a.display_name.clone())
                .unwrap_or_default(),
            self.created_at.format("%Y-%m-%d %H:%M").to_string(),
            self.content.chars().take(40).collect(),
        ]
    }
}

/// Document backlink (inbound link), mirroring `project_backlink` from the MCP.
///
/// `source_document_id` (UUID) is dropped in favour of `source_slug` when present.
/// `display_title` is the rendered title preferred for display.
#[derive(Debug, Serialize)]
pub(crate) struct DocBacklinkProjection {
    pub(crate) source_title: String,
    pub(crate) display_title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) source_slug: Option<String>,
}

impl From<BacklinkDto> for DocBacklinkProjection {
    fn from(b: BacklinkDto) -> Self {
        Self {
            source_title: b.source_title,
            display_title: b.display_title,
            source_slug: b.source_slug,
        }
    }
}

impl TableRow for DocBacklinkProjection {
    fn headers() -> &'static [&'static str] {
        &["Source Title", "Display Title", "Slug"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.source_title.clone(),
            self.display_title.clone(),
            self.source_slug.clone().unwrap_or_default(),
        ]
    }
}

// ---------------------------------------------------------------------------
// Attachment projection (docs + tasks)
// ---------------------------------------------------------------------------

/// CLI projection for a document or task attachment.
///
/// Normalises `file_name → filename` and `size_bytes → size` for a uniform
/// shape regardless of whether the source is a document or task attachment.
#[derive(Debug, Serialize)]
pub(crate) struct AttachProjection {
    pub(crate) id: Uuid,
    pub(crate) filename: String,
    pub(crate) content_type: String,
    pub(crate) size: i64,
    pub(crate) created_at: DateTime<Utc>,
}

impl From<AttachmentDto> for AttachProjection {
    fn from(dto: AttachmentDto) -> Self {
        Self {
            id: dto.id,
            filename: dto.file_name,
            content_type: dto.content_type,
            size: dto.size_bytes,
            created_at: dto.created_at,
        }
    }
}

impl From<TaskAttachmentDto> for AttachProjection {
    fn from(dto: TaskAttachmentDto) -> Self {
        Self {
            id: dto.id,
            filename: dto.file_name,
            content_type: dto.content_type,
            size: dto.size_bytes,
            created_at: dto.created_at,
        }
    }
}

impl TableRow for AttachProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Filename", "Content-Type", "Size", "Created"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.id.to_string(),
            self.filename.clone(),
            self.content_type.clone(),
            self.size.to_string(),
            self.created_at.format("%Y-%m-%d").to_string(),
        ]
    }
}

/// Workspace-scoped activity entry, mirroring `project_workspace_activity_entry` from the MCP.
///
/// Adds `task_readable_id` so callers can navigate to the originating task.
/// `id` (UUID) and `task_id` are dropped.
#[derive(Debug, Serialize)]
pub(crate) struct WorkspaceActivityProjection {
    pub(crate) task_readable_id: String,
    pub(crate) kind: String,
    pub(crate) actor: AssigneeProjection,
    pub(crate) payload: serde_json::Value,
    pub(crate) created_at: DateTime<Utc>,
}

impl From<ActivityEntryDto> for WorkspaceActivityProjection {
    fn from(entry: ActivityEntryDto) -> Self {
        Self {
            task_readable_id: entry.task_readable_id,
            kind: entry.kind,
            actor: AssigneeProjection::from(entry.actor),
            payload: entry.payload,
            created_at: entry.created_at,
        }
    }
}

impl TableRow for WorkspaceActivityProjection {
    fn headers() -> &'static [&'static str] {
        &["Task", "Kind", "Actor", "Created At"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.task_readable_id.clone(),
            self.kind.clone(),
            self.actor
                .display_name
                .clone()
                .unwrap_or_else(|| self.actor.type_.clone()),
            self.created_at.format("%Y-%m-%d %H:%M").to_string(),
        ]
    }
}

/// Minimal deletion confirmation used when no meaningful resource ID needs surfacing.
///
/// Used for `assignees remove` and similar operations where the caller already holds
/// the identifier and only needs to confirm that the operation succeeded.
#[derive(Debug, Serialize)]
pub(crate) struct DeletedProjection {
    pub(crate) deleted: bool,
}

impl TableRow for DeletedProjection {
    fn headers() -> &'static [&'static str] {
        &["Deleted"]
    }

    fn row(&self) -> Vec<String> {
        vec![self.deleted.to_string()]
    }
}

/// Deletion confirmation for resources identified by a UUID `id`.
///
/// Used for groups and API-key revocation where the natural identifier is
/// the resource UUID rather than a human-readable slug or readable_id.
#[derive(Debug, Serialize)]
pub(crate) struct DeleteByIdProjection {
    pub(crate) deleted: bool,
    pub(crate) id: Uuid,
}

impl TableRow for DeleteByIdProjection {
    fn headers() -> &'static [&'static str] {
        &["Deleted", "ID"]
    }

    fn row(&self) -> Vec<String> {
        vec![self.deleted.to_string(), self.id.to_string()]
    }
}

// ---------------------------------------------------------------------------
// Admin projections (Batch 5-A)
// ---------------------------------------------------------------------------

/// User account projection, mirroring all non-secret fields from `UserDto`.
///
/// Optional fields (`email`, `disabled_at`, `activated_at`) are omitted when
/// absent so the JSON output stays minimal for users in the common case.
#[derive(Debug, Serialize)]
pub(crate) struct UserProjection {
    pub(crate) id: Uuid,
    pub(crate) username: String,
    pub(crate) display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) email: Option<String>,
    pub(crate) is_root: bool,
    pub(crate) is_system_admin: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) disabled_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) activated_at: Option<DateTime<Utc>>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

impl From<UserDto> for UserProjection {
    fn from(u: UserDto) -> Self {
        Self {
            id: u.id,
            username: u.username,
            display_name: u.display_name,
            email: u.email,
            is_root: u.is_root,
            is_system_admin: u.is_system_admin,
            disabled_at: u.disabled_at,
            activated_at: u.activated_at,
            created_at: u.created_at,
            updated_at: u.updated_at,
        }
    }
}

impl TableRow for UserProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Username", "Display Name", "Root", "Admin", "Status"]
    }

    fn row(&self) -> Vec<String> {
        let status = if self.disabled_at.is_some() {
            "disabled"
        } else if self.activated_at.is_none() {
            "pending"
        } else {
            "active"
        };
        vec![
            self.id.to_string(),
            self.username.clone(),
            self.display_name.clone(),
            self.is_root.to_string(),
            self.is_system_admin.to_string(),
            status.to_owned(),
        ]
    }
}

/// Projection returned by `users create`, which deliberately surfaces the
/// one-time `activation_link` (not stored server-side after the response).
#[derive(Debug, Serialize)]
pub(crate) struct UserCreatedProjection {
    pub(crate) id: Uuid,
    pub(crate) username: String,
    pub(crate) display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) email: Option<String>,
    pub(crate) is_root: bool,
    pub(crate) is_system_admin: bool,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
    /// Single-use activation link. Show this to the invitee exactly once.
    pub(crate) activation_link: String,
}

impl From<CreateUserResponse> for UserCreatedProjection {
    fn from(r: CreateUserResponse) -> Self {
        Self {
            id: r.user.id,
            username: r.user.username,
            display_name: r.user.display_name,
            email: r.user.email,
            is_root: r.user.is_root,
            is_system_admin: r.user.is_system_admin,
            created_at: r.user.created_at,
            updated_at: r.user.updated_at,
            activation_link: r.activation_link,
        }
    }
}

impl TableRow for UserCreatedProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Username", "Display Name", "Activation Link"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.id.to_string(),
            self.username.clone(),
            self.display_name.clone(),
            self.activation_link.clone(),
        ]
    }
}

/// Projection for a freshly issued activation link (`users regenerate-link`).
///
/// `activation_link` is the single-use path shown exactly once.
#[derive(Debug, Serialize)]
pub(crate) struct ActivationLinkProjection {
    pub(crate) activation_link: String,
}

impl From<ActivationLinkResponse> for ActivationLinkProjection {
    fn from(r: ActivationLinkResponse) -> Self {
        Self {
            activation_link: r.activation_link,
        }
    }
}

impl TableRow for ActivationLinkProjection {
    fn headers() -> &'static [&'static str] {
        &["Activation Link"]
    }

    fn row(&self) -> Vec<String> {
        vec![self.activation_link.clone()]
    }
}

/// Workspace membership for a specific user (`users memberships`).
#[derive(Debug, Serialize)]
pub(crate) struct UserMembershipProjection {
    pub(crate) workspace_slug: String,
    pub(crate) workspace_name: String,
    pub(crate) role: String,
}

impl From<UserMembershipDto> for UserMembershipProjection {
    fn from(m: UserMembershipDto) -> Self {
        Self {
            workspace_slug: m.workspace_slug,
            workspace_name: m.workspace_name,
            role: m.role,
        }
    }
}

impl TableRow for UserMembershipProjection {
    fn headers() -> &'static [&'static str] {
        &["Workspace Slug", "Workspace Name", "Role"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.workspace_slug.clone(),
            self.workspace_name.clone(),
            self.role.clone(),
        ]
    }
}

/// API key summary projection for `api-keys list`. Does not include the secret.
///
/// `type` is a reserved keyword in Rust; the field is renamed via serde.
#[derive(Debug, Serialize)]
pub(crate) struct ApiKeyProjection {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    #[serde(rename = "type")]
    pub(crate) type_: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) expires_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) last_used_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) revoked_at: Option<DateTime<Utc>>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) is_global: bool,
    /// Capability scopes in canonical `family:action` form.
    pub(crate) scopes: Vec<String>,
}

/// Renders a wire capability scope to its canonical `family:action` string using
/// the type's own serde mapping, avoiding a hand-maintained variant match.
fn scope_wire_name(scope: &ApiKeyScope) -> String {
    serde_json::to_value(scope)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

impl From<ApiKeyDto> for ApiKeyProjection {
    fn from(k: ApiKeyDto) -> Self {
        Self {
            id: k.id,
            name: k.name,
            type_: k.r#type,
            expires_at: k.expires_at,
            last_used_at: k.last_used_at,
            revoked_at: k.revoked_at,
            created_at: k.created_at,
            is_global: k.is_global,
            scopes: k.scopes.iter().map(scope_wire_name).collect(),
        }
    }
}

impl TableRow for ApiKeyProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Name", "Type", "Global", "Scopes", "Created At"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.id.to_string(),
            self.name.clone(),
            self.type_.clone(),
            self.is_global.to_string(),
            self.scopes.join(","),
            self.created_at.format("%Y-%m-%d").to_string(),
        ]
    }
}

/// Projection for a newly created API key (`api-keys create`).
///
/// Deliberately surfaces the one-time plaintext `secret` — this is the
/// intended output of key creation, not a leak.
#[derive(Debug, Serialize)]
pub(crate) struct ApiKeyCreatedProjection {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    /// One-time plaintext secret. Shown at creation only; not stored server-side.
    pub(crate) secret: String,
    #[serde(rename = "type")]
    pub(crate) type_: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) expires_at: Option<DateTime<Utc>>,
    pub(crate) created_at: DateTime<Utc>,
}

impl From<ApiKeyCreated> for ApiKeyCreatedProjection {
    fn from(k: ApiKeyCreated) -> Self {
        Self {
            id: k.id,
            name: k.name,
            secret: k.secret,
            type_: k.r#type,
            expires_at: k.expires_at,
            created_at: k.created_at,
        }
    }
}

impl TableRow for ApiKeyCreatedProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Name", "Secret", "Type", "Created At"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.id.to_string(),
            self.name.clone(),
            self.secret.clone(),
            self.type_.clone(),
            self.created_at.format("%Y-%m-%d").to_string(),
        ]
    }
}

/// API key grant projection for `api-keys grants`.
///
/// `granted_by` is omitted when absent (legacy grants without attribution).
#[derive(Debug, Serialize)]
pub(crate) struct ApiKeyGrantProjection {
    pub(crate) id: Uuid,
    pub(crate) role: String,
    pub(crate) resource_kind: String,
    pub(crate) resource_label: String,
    pub(crate) workspace_slug: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) project_slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) granted_by: Option<GrantedByDto>,
}

impl From<ApiKeyGrantDto> for ApiKeyGrantProjection {
    fn from(g: ApiKeyGrantDto) -> Self {
        Self {
            id: g.id,
            role: g.role,
            resource_kind: g.resource_kind,
            resource_label: g.resource_label,
            workspace_slug: g.workspace_slug,
            project_slug: g.project_slug,
            granted_by: g.granted_by,
        }
    }
}

impl TableRow for ApiKeyGrantProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Role", "Resource Kind", "Resource Label", "Workspace"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.id.to_string(),
            self.role.clone(),
            self.resource_kind.clone(),
            self.resource_label.clone(),
            self.workspace_slug.clone(),
        ]
    }
}

/// Group projection for `groups list` and `groups create`.
///
/// `workspace_id` and `created_by` are internal identifiers dropped from the
/// projection; `name` and timestamps are sufficient for display.
#[derive(Debug, Serialize)]
pub(crate) struct GroupProjection {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

impl From<GroupDto> for GroupProjection {
    fn from(g: GroupDto) -> Self {
        Self {
            id: g.id,
            name: g.name,
            created_at: g.created_at,
            updated_at: g.updated_at,
        }
    }
}

impl TableRow for GroupProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Name", "Created At", "Updated At"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.id.to_string(),
            self.name.clone(),
            self.created_at.format("%Y-%m-%d").to_string(),
            self.updated_at.format("%Y-%m-%d").to_string(),
        ]
    }
}

/// Group member projection for `groups members` and `groups add-member`.
#[derive(Debug, Serialize)]
pub(crate) struct GroupMemberProjection {
    pub(crate) group_id: Uuid,
    pub(crate) user_id: Uuid,
    pub(crate) created_at: DateTime<Utc>,
}

impl From<GroupMemberDto> for GroupMemberProjection {
    fn from(m: GroupMemberDto) -> Self {
        Self {
            group_id: m.group_id,
            user_id: m.user_id,
            created_at: m.created_at,
        }
    }
}

impl TableRow for GroupMemberProjection {
    fn headers() -> &'static [&'static str] {
        &["Group ID", "User ID", "Added At"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.group_id.to_string(),
            self.user_id.to_string(),
            self.created_at.format("%Y-%m-%d").to_string(),
        ]
    }
}

// ---------------------------------------------------------------------------
// Workspace config projections (Batch 5-B)
// ---------------------------------------------------------------------------

/// Status template projection, mirroring the MCP `project_status_template` shape.
///
/// `workspace_id` and `created_at` are dropped. `color` is omitted when absent.
/// `position_key` is retained so callers can use `before`/`after` anchors.
#[derive(Debug, Serialize)]
pub(crate) struct StatusTemplateProjection {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    pub(crate) position_key: String,
    pub(crate) updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) color: Option<String>,
}

impl From<StatusTemplateDto> for StatusTemplateProjection {
    fn from(t: StatusTemplateDto) -> Self {
        Self {
            id: t.id,
            name: t.name,
            position_key: t.position_key,
            updated_at: t.updated_at,
            color: t.color,
        }
    }
}

impl TableRow for StatusTemplateProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Name", "Color", "Position", "Updated At"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.id.to_string(),
            self.name.clone(),
            self.color.clone().unwrap_or_default(),
            self.position_key.clone(),
            self.updated_at.format("%Y-%m-%d").to_string(),
        ]
    }
}

/// Saved search projection, mirroring the MCP `project_saved_search` shape.
///
/// `workspace_id` and timestamps are dropped; `query` is retained so the
/// caller can inspect or reuse the filter string directly.
#[derive(Debug, Serialize)]
pub(crate) struct SavedSearchProjection {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    pub(crate) query: String,
}

impl From<SavedSearchDto> for SavedSearchProjection {
    fn from(s: SavedSearchDto) -> Self {
        Self {
            id: s.id,
            name: s.name,
            query: s.query,
        }
    }
}

impl TableRow for SavedSearchProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Name", "Query"]
    }

    fn row(&self) -> Vec<String> {
        vec![self.id.to_string(), self.name.clone(), self.query.clone()]
    }
}

/// Task view projection, mirroring the MCP `project_task_view` shape.
///
/// `workspace_id` and timestamps are dropped. `filters` passes through the
/// DTO verbatim — its `skip_serializing_if` guards keep absent fields out of
/// the JSON output.
#[derive(Debug, Serialize)]
pub(crate) struct TaskViewProjection {
    pub(crate) id: Uuid,
    pub(crate) name: String,
    pub(crate) filters: TaskViewFiltersDto,
}

impl From<TaskViewDto> for TaskViewProjection {
    fn from(v: TaskViewDto) -> Self {
        Self {
            id: v.id,
            name: v.name,
            filters: v.filters,
        }
    }
}

impl TableRow for TaskViewProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Name", "Filters"]
    }

    fn row(&self) -> Vec<String> {
        let filters_json = serde_json::to_string(&self.filters).unwrap_or_default();
        vec![self.id.to_string(), self.name.clone(), filters_json]
    }
}

/// Property definition projection for `property-definitions list` and `create`.
///
/// Exposes all DTO fields except `workspace_id` (not present in the DTO).
/// `options` is omitted when absent (non-select kinds have no options array).
#[derive(Debug, Serialize)]
pub(crate) struct PropertyDefinitionProjection {
    pub(crate) id: Uuid,
    pub(crate) key: String,
    pub(crate) name: String,
    pub(crate) kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) options: Option<serde_json::Value>,
    pub(crate) applies_to: String,
    pub(crate) created_at: DateTime<Utc>,
}

impl From<PropertyDefinitionDto> for PropertyDefinitionProjection {
    fn from(p: PropertyDefinitionDto) -> Self {
        Self {
            id: p.id,
            key: p.key,
            name: p.name,
            kind: p.kind,
            options: p.options,
            applies_to: p.applies_to,
            created_at: p.created_at,
        }
    }
}

impl TableRow for PropertyDefinitionProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Key", "Name", "Kind", "Applies To", "Created At"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.id.to_string(),
            self.key.clone(),
            self.name.clone(),
            self.kind.clone(),
            self.applies_to.clone(),
            self.created_at.format("%Y-%m-%d").to_string(),
        ]
    }
}

// ---------------------------------------------------------------------------
// Permission grant projections (Batch 5-C)
// ---------------------------------------------------------------------------

/// Projection for a permission grant (`grants workspace/project list`, `create`).
///
/// Mirrors `GrantDto` fields verbatim. `principal` carries the grantee type
/// and UUID. `id` is retained so callers can use it with `revoke`.
#[derive(Debug, Serialize)]
pub(crate) struct GrantProjection {
    pub(crate) id: Uuid,
    pub(crate) principal: GrantPrincipal,
    pub(crate) role: String,
    pub(crate) created_at: DateTime<Utc>,
}

impl From<GrantDto> for GrantProjection {
    fn from(g: GrantDto) -> Self {
        Self {
            id: g.id,
            principal: g.principal,
            role: g.role,
            created_at: g.created_at,
        }
    }
}

impl TableRow for GrantProjection {
    fn headers() -> &'static [&'static str] {
        &["ID", "Principal Type", "Principal ID", "Role", "Created At"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.id.to_string(),
            self.principal.r#type.clone(),
            self.principal.id.to_string(),
            self.role.clone(),
            self.created_at.format("%Y-%m-%d").to_string(),
        ]
    }
}

// ---------------------------------------------------------------------------
// Audit entry projections (Batch 5-C)
// ---------------------------------------------------------------------------

/// Actor sub-projection for `AuditEntryProjection`, mirroring the actor
/// object in the MCP `project_audit_entry` shape.
///
/// `id` is dropped (internal UUID with no client-navigation value).
/// Optional fields are omitted when absent.
#[derive(Debug, Serialize)]
pub(crate) struct AuditActorProjection {
    #[serde(rename = "type")]
    pub(crate) type_: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) key_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) account_status: Option<String>,
}

/// Projection for a security audit log entry, mirroring the MCP
/// `project_audit_entry` shape exactly.
///
/// `id` and `workspace_id` are dropped — internal identifiers with no
/// agent-navigation value. `metadata` is passed through verbatim.
#[derive(Debug, Serialize)]
pub(crate) struct AuditEntryProjection {
    pub(crate) actor: AuditActorProjection,
    pub(crate) action: String,
    pub(crate) target_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) target_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) target_label: Option<String>,
    pub(crate) metadata: serde_json::Value,
    pub(crate) created_at: DateTime<Utc>,
}

impl From<AuditEntryDto> for AuditEntryProjection {
    fn from(entry: AuditEntryDto) -> Self {
        let actor = AuditActorProjection {
            type_: entry.actor.r#type,
            display_name: entry.actor.display_name,
            key_type: entry.actor.key_type,
            account_status: entry.actor.account_status,
        };
        Self {
            actor,
            action: entry.action,
            target_type: entry.target_type,
            target_id: entry.target_id,
            target_label: entry.target_label,
            metadata: entry.metadata,
            created_at: entry.created_at,
        }
    }
}

impl TableRow for AuditEntryProjection {
    fn headers() -> &'static [&'static str] {
        &["Action", "Actor", "Target Type", "Created At"]
    }

    fn row(&self) -> Vec<String> {
        let actor_label = self
            .actor
            .display_name
            .clone()
            .unwrap_or_else(|| self.actor.type_.clone());
        vec![
            self.action.clone(),
            actor_label,
            self.target_type.clone(),
            self.created_at.format("%Y-%m-%d %H:%M").to_string(),
        ]
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    /// Asserts that a serialized `serde_json::Value` contains all `required`
    /// keys and only keys from `required ∪ optional`. Unknown keys cause a panic.
    pub(crate) fn assert_projection_fields(
        v: &serde_json::Value,
        required: &[&str],
        optional: &[&str],
    ) {
        let obj = v
            .as_object()
            .expect("projection must serialize to a JSON object");

        for key in required {
            assert!(
                obj.contains_key(*key),
                "required field '{key}' missing from projection: {obj:?}"
            );
        }

        let allowed: std::collections::HashSet<&str> =
            required.iter().chain(optional.iter()).copied().collect();

        for key in obj.keys() {
            assert!(
                allowed.contains(key.as_str()),
                "unexpected field '{key}' in projection (not in required or optional)"
            );
        }
    }

    fn make_search_hit(kind: SearchKindDto, readable_id: Option<&str>) -> SearchHitDto {
        SearchHitDto {
            id: Uuid::now_v7(),
            kind,
            readable_id: readable_id.map(str::to_owned),
            title: "Test hit".to_owned(),
            snippet: None,
            score: 0.9,
            updated_at: Utc::now(),
            project_slug: None,
            column_name: None,
        }
    }

    #[test]
    fn search_hit_projection_contract_required_and_optional_fields() {
        let dto = make_search_hit(SearchKindDto::Task, Some("ATL-1"));
        let proj = SearchHitProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();

        assert_projection_fields(
            &value,
            &["id", "kind", "title", "score", "updated_at"],
            &["readable_id", "snippet", "project_slug", "column_name"],
        );
    }

    #[test]
    fn search_hit_task_kind_serializes_as_lowercase_string() {
        let dto = make_search_hit(SearchKindDto::Task, None);
        let proj = SearchHitProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["kind"], "task");
    }

    #[test]
    fn search_hit_document_kind_serializes_as_lowercase_string() {
        let dto = make_search_hit(SearchKindDto::Document, None);
        let proj = SearchHitProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["kind"], "document");
    }

    #[test]
    fn search_hit_optional_fields_absent_when_none() {
        let dto = make_search_hit(SearchKindDto::Document, None);
        let proj = SearchHitProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();

        assert!(
            value.get("readable_id").is_none(),
            "readable_id must be absent when None"
        );
        assert!(
            value.get("snippet").is_none(),
            "snippet must be absent when None"
        );
        assert!(
            value.get("project_slug").is_none(),
            "project_slug must be absent when None"
        );
        assert!(
            value.get("column_name").is_none(),
            "column_name must be absent when None"
        );
    }

    #[test]
    fn search_hit_optional_fields_present_when_some() {
        let mut dto = make_search_hit(SearchKindDto::Task, Some("ATL-42"));
        dto.snippet = Some("highlighted text".to_owned());
        dto.project_slug = Some("my-project".to_owned());
        dto.column_name = Some("In Progress".to_owned());

        let proj = SearchHitProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();

        assert_eq!(value["readable_id"], "ATL-42");
        assert_eq!(value["snippet"], "highlighted text");
        assert_eq!(value["project_slug"], "my-project");
        assert_eq!(value["column_name"], "In Progress");
    }

    #[test]
    fn envelope_serializes_with_items_cursor_and_has_more() {
        let items: Vec<serde_json::Value> = vec![serde_json::json!({"x": 1})];
        let env = Envelope {
            items,
            next_cursor: Some("cursor123".to_owned()),
            has_more: true,
        };
        let value = serde_json::to_value(&env).unwrap();
        assert_eq!(value["has_more"], true);
        assert_eq!(value["next_cursor"], "cursor123");
        assert!(value["items"].is_array());
    }

    #[test]
    fn envelope_next_cursor_is_null_when_none() {
        let env: Envelope<serde_json::Value> = Envelope {
            items: vec![],
            next_cursor: None,
            has_more: false,
        };
        let value = serde_json::to_value(&env).unwrap();
        assert!(value["next_cursor"].is_null());
    }

    // -----------------------------------------------------------------------
    // Task projections (T27-T28)
    // -----------------------------------------------------------------------

    fn make_actor_dto() -> atlas_api::dtos::documents::ActorDto {
        atlas_api::dtos::documents::ActorDto {
            r#type: "user".to_owned(),
            id: Uuid::now_v7(),
            display_name: Some("Alice".to_owned()),
            key_type: None,
            account_status: None,
        }
    }

    fn make_task_dto(board_name: &str, column_name: &str) -> TaskDto {
        TaskDto {
            id: Uuid::now_v7(),
            workspace_id: Uuid::now_v7(),
            project_id: Uuid::now_v7(),
            board_id: Uuid::now_v7(),
            column_id: Uuid::now_v7(),
            parent_task_id: None,
            readable_id: "ATL-1".to_owned(),
            title: "Test task".to_owned(),
            description: String::new(),
            priority: Some("high".to_owned()),
            due_date: None,
            estimate: Some(3),
            labels: vec!["rust".to_owned()],
            properties: None,
            created_by: make_actor_dto(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            board_name: board_name.to_owned(),
            column_name: column_name.to_owned(),
        }
    }

    fn make_task_summary_dto() -> TaskSummaryDto {
        TaskSummaryDto {
            id: Uuid::now_v7(),
            readable_id: "ATL-2".to_owned(),
            board_id: Uuid::now_v7(),
            column_id: Uuid::now_v7(),
            title: "Summary task".to_owned(),
            priority: Some("low".to_owned()),
            estimate: None,
            labels: vec![],
            assignees: vec![make_actor_dto()],
            board_name: "Dev Board".to_owned(),
            column_name: "In Progress".to_owned(),
            subtask_count: 0,
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn assignee_projection_serializes_type_as_json_key() {
        let dto = make_actor_dto();
        let proj = AssigneeProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["type"], "user");
        assert_eq!(value["display_name"], "Alice");
    }

    #[test]
    fn assignee_projection_contract_fields() {
        let dto = make_actor_dto();
        let proj = AssigneeProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(&value, &["type", "display_name"], &[]);
    }

    #[test]
    fn task_summary_projection_contract_fields() {
        let dto = make_task_summary_dto();
        let proj = TaskSummaryProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(
            &value,
            &[
                "readable_id",
                "title",
                "board_name",
                "column_name",
                "labels",
                "assignees",
                "updated_at",
            ],
            &["priority", "estimate"],
        );
    }

    #[test]
    fn task_summary_assignees_contain_type_and_display_name() {
        let dto = make_task_summary_dto();
        let proj = TaskSummaryProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        let assignees = value["assignees"].as_array().unwrap();
        assert_eq!(assignees.len(), 1);
        assert_eq!(assignees[0]["type"], "user");
        assert_eq!(assignees[0]["display_name"], "Alice");
    }

    #[test]
    fn task_compact_projection_contract_fields() {
        let dto = make_task_dto("Dev Board", "To Do");
        let proj = TaskCompactProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(
            &value,
            &["readable_id", "title", "priority", "labels", "updated_at"],
            &[
                "board_name",
                "column_name",
                "estimate",
                "due_date",
                "parent_task_id",
            ],
        );
    }

    #[test]
    fn task_compact_board_and_column_omitted_when_empty() {
        let dto = make_task_dto("", "");
        let proj = TaskCompactProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(
            value.get("board_name").is_none(),
            "board_name must be absent when empty"
        );
        assert!(
            value.get("column_name").is_none(),
            "column_name must be absent when empty"
        );
    }

    #[test]
    fn task_compact_board_and_column_present_when_non_empty() {
        let dto = make_task_dto("Dev Board", "To Do");
        let proj = TaskCompactProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["board_name"], "Dev Board");
        assert_eq!(value["column_name"], "To Do");
    }

    #[test]
    fn task_full_projection_contract_fields() {
        let dto = make_task_dto("Dev Board", "To Do");
        let proj = TaskFullProjection::new(dto, Ok(vec![]), Ok(vec![]), Ok(vec![]));
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(
            &value,
            &[
                "readable_id",
                "title",
                "description",
                "labels",
                "updated_at",
            ],
            &[
                "board_name",
                "column_name",
                "priority",
                "estimate",
                "due_date",
                "parent_task_id",
                "references",
                "references_error",
                "subtasks",
                "subtasks_error",
                "assignees",
                "assignees_error",
            ],
        );
    }

    #[test]
    fn task_full_projection_sub_resource_error_replaces_value() {
        let dto = make_task_dto("", "");
        let proj = TaskFullProjection::new(
            dto,
            Err("list_references failed: timeout".to_owned()),
            Ok(vec![]),
            Ok(vec![]),
        );
        let value = serde_json::to_value(&proj).unwrap();
        assert!(
            value.get("references").is_none(),
            "references must be absent on error"
        );
        assert!(
            value["references_error"].is_string(),
            "references_error must be set"
        );
        assert!(
            value.get("subtasks_error").is_none(),
            "subtasks_error must be absent on success"
        );
        assert!(
            value["subtasks"].is_array(),
            "subtasks must be present on success"
        );
    }

    #[test]
    fn delete_task_projection_serializes_readable_id_not_id() {
        let proj = DeleteTaskProjection {
            deleted: true,
            readable_id: "ATL-1".to_owned(),
        };
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["deleted"], true);
        assert_eq!(value["readable_id"], "ATL-1");
        assert!(
            value.get("id").is_none(),
            "must not contain a generic 'id' key"
        );
        assert_projection_fields(&value, &["deleted", "readable_id"], &[]);
    }

    // -----------------------------------------------------------------------
    // Document projections (T36)
    // -----------------------------------------------------------------------

    fn make_document_dto(slug: Option<&str>, folder_id: Option<Uuid>) -> DocumentDto {
        DocumentDto {
            id: Uuid::now_v7(),
            workspace_id: Uuid::now_v7(),
            project_id: Some(Uuid::now_v7()),
            folder_id,
            slug: slug.map(str::to_owned),
            title: "Test Document".to_owned(),
            content: "# Hello\nWorld".to_owned(),
            head_revision_id: Uuid::now_v7(),
            head_seq: 5,
            frontmatter: serde_json::json!({"tags": ["rust"]}),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_document_summary_dto(slug: Option<&str>) -> DocumentSummaryDto {
        DocumentSummaryDto {
            id: Uuid::now_v7(),
            slug: slug.map(str::to_owned),
            title: "Summary Doc".to_owned(),
            folder_id: None,
            head_seq: 3,
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn doc_compact_projection_contract_fields() {
        let dto = make_document_dto(Some("my-doc"), None);
        let proj = DocCompactProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(
            &value,
            &[
                "id",
                "slug",
                "title",
                "head_revision_id",
                "head_seq",
                "updated_at",
                "folder_id",
                "project_id",
            ],
            &[],
        );
    }

    #[test]
    fn doc_compact_projection_slug_null_when_none() {
        let dto = make_document_dto(None, None);
        let proj = DocCompactProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(
            value["slug"].is_null(),
            "slug must be null (not absent) when None"
        );
    }

    #[test]
    fn doc_compact_projection_folder_id_null_when_none() {
        let dto = make_document_dto(Some("x"), None);
        let proj = DocCompactProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(
            value["folder_id"].is_null(),
            "folder_id must be null when None"
        );
    }

    #[test]
    fn doc_full_projection_contract_fields() {
        let dto = make_document_dto(Some("full-doc"), None);
        let proj = DocFullProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(
            &value,
            &[
                "id",
                "slug",
                "title",
                "head_revision_id",
                "head_seq",
                "updated_at",
                "folder_id",
                "project_id",
                "content",
                "frontmatter",
            ],
            &[],
        );
    }

    #[test]
    fn doc_full_projection_content_and_frontmatter_present() {
        let dto = make_document_dto(Some("doc"), None);
        let proj = DocFullProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(value["content"].is_string());
        assert!(value["frontmatter"].is_object());
    }

    #[test]
    fn delete_doc_projection_serializes_slug_not_id() {
        let proj = DeleteDocProjection {
            deleted: true,
            slug: "my-doc".to_owned(),
        };
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["deleted"], true);
        assert_eq!(value["slug"], "my-doc");
        assert!(
            value.get("id").is_none(),
            "must not contain a generic 'id' key"
        );
        assert_projection_fields(&value, &["deleted", "slug"], &[]);
    }

    #[test]
    fn doc_summary_projection_contract_fields_required() {
        let dto = make_document_summary_dto(None);
        let proj = DocSummaryProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(
            &value,
            &["id", "title", "head_seq", "updated_at"],
            &["slug", "folder_id"],
        );
    }

    #[test]
    fn doc_summary_projection_slug_absent_when_none() {
        let dto = make_document_summary_dto(None);
        let proj = DocSummaryProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(
            value.get("slug").is_none(),
            "slug must be absent (not null) when None"
        );
    }

    #[test]
    fn doc_summary_projection_slug_present_when_some() {
        let dto = make_document_summary_dto(Some("notes"));
        let proj = DocSummaryProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["slug"], "notes");
    }

    // -----------------------------------------------------------------------
    // Structure projections (T41 — WU-17)
    // -----------------------------------------------------------------------

    fn make_workspace_dto() -> WorkspaceDto {
        WorkspaceDto {
            id: Uuid::now_v7(),
            name: "My Workspace".to_owned(),
            slug: "my-ws".to_owned(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_project_dto(visibility_role: Option<&str>) -> ProjectDto {
        ProjectDto {
            id: Uuid::now_v7(),
            workspace_id: Uuid::now_v7(),
            name: "Atlas".to_owned(),
            slug: "atlas".to_owned(),
            task_prefix: "ATL".to_owned(),
            visibility: "workspace".to_owned(),
            visibility_role: visibility_role.map(str::to_owned),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_board_summary_dto() -> BoardSummaryDto {
        BoardSummaryDto {
            id: Uuid::now_v7(),
            name: "Dev Board".to_owned(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_column_dto(color: Option<&str>) -> ColumnDto {
        ColumnDto {
            id: Uuid::now_v7(),
            board_id: Uuid::now_v7(),
            name: "To Do".to_owned(),
            position_key: "a0".to_owned(),
            color: color.map(str::to_owned),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_tag_dto(color: Option<&str>) -> TagDto {
        TagDto {
            id: Uuid::now_v7(),
            workspace_id: Uuid::now_v7(),
            name: "rust".to_owned(),
            color: color.map(str::to_owned),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_principal_dto() -> PrincipalDto {
        PrincipalDto {
            principal_type: "user".to_owned(),
            id: Uuid::now_v7(),
            display: "Alice".to_owned(),
            key_type: None,
            role: Some("member".to_owned()),
            account_status: Some("active".to_owned()),
        }
    }

    fn make_folder_dto(parent: Option<Uuid>) -> FolderDto {
        FolderDto {
            id: Uuid::now_v7(),
            workspace_id: Uuid::now_v7(),
            project_id: Some(Uuid::now_v7()),
            parent_folder_id: parent,
            name: "Notes".to_owned(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn workspace_projection_contract_fields() {
        let dto = make_workspace_dto();
        let proj = WorkspaceProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(&value, &["id", "name", "slug", "updated_at"], &[]);
    }

    #[test]
    fn workspace_projection_no_created_at() {
        let dto = make_workspace_dto();
        let proj = WorkspaceProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(
            value.get("created_at").is_none(),
            "created_at must be dropped"
        );
    }

    #[test]
    fn project_projection_contract_required_and_optional() {
        let dto = make_project_dto(None);
        let proj = ProjectProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(
            &value,
            &[
                "id",
                "name",
                "slug",
                "task_prefix",
                "visibility",
                "updated_at",
            ],
            &["visibility_role"],
        );
    }

    #[test]
    fn project_projection_visibility_role_absent_when_none() {
        let dto = make_project_dto(None);
        let proj = ProjectProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(
            value.get("visibility_role").is_none(),
            "visibility_role must be absent when None"
        );
    }

    #[test]
    fn project_projection_visibility_role_present_when_some() {
        let dto = make_project_dto(Some("editor"));
        let proj = ProjectProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["visibility_role"], "editor");
    }

    #[test]
    fn board_projection_contract_fields() {
        let dto = make_board_summary_dto();
        let proj = BoardProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(&value, &["id", "name", "updated_at"], &[]);
    }

    #[test]
    fn column_projection_contract_fields() {
        let dto = make_column_dto(None);
        let proj = ColumnProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(&value, &["id", "name"], &["color"]);
    }

    #[test]
    fn column_projection_color_absent_when_none() {
        let dto = make_column_dto(None);
        let proj = ColumnProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(
            value.get("color").is_none(),
            "color must be absent when None"
        );
    }

    #[test]
    fn column_projection_color_present_when_some() {
        let dto = make_column_dto(Some("#FF5733"));
        let proj = ColumnProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["color"], "#FF5733");
    }

    #[test]
    fn tag_projection_contract_fields() {
        let dto = make_tag_dto(None);
        let proj = TagProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(&value, &["id", "name"], &["color"]);
    }

    #[test]
    fn tag_projection_color_absent_when_none() {
        let dto = make_tag_dto(None);
        let proj = TagProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(
            value.get("color").is_none(),
            "color must be absent when None"
        );
    }

    #[test]
    fn tag_projection_color_present_when_some() {
        let dto = make_tag_dto(Some("#3B82F6"));
        let proj = TagProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["color"], "#3B82F6");
    }

    #[test]
    fn member_projection_contract_fields() {
        let dto = make_principal_dto();
        let proj = MemberProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(&value, &["principal_type", "id", "display"], &[]);
    }

    #[test]
    fn member_projection_drops_role_and_key_type() {
        let dto = make_principal_dto();
        let proj = MemberProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(value.get("role").is_none(), "role must be dropped");
        assert!(value.get("key_type").is_none(), "key_type must be dropped");
        assert!(
            value.get("account_status").is_none(),
            "account_status must be dropped"
        );
    }

    #[test]
    fn folder_projection_contract_fields() {
        let dto = make_folder_dto(None);
        let proj = FolderProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(&value, &["id", "name", "updated_at"], &["parent_folder_id"]);
    }

    #[test]
    fn folder_projection_parent_absent_when_none() {
        let dto = make_folder_dto(None);
        let proj = FolderProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(
            value.get("parent_folder_id").is_none(),
            "parent_folder_id must be absent when None"
        );
    }

    #[test]
    fn folder_projection_parent_present_when_some() {
        let parent = Uuid::now_v7();
        let dto = make_folder_dto(Some(parent));
        let proj = FolderProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["parent_folder_id"], parent.to_string());
    }

    // -----------------------------------------------------------------------
    // Graph / activity projections (T48 — WU-21)
    // -----------------------------------------------------------------------

    fn make_reference_dto() -> atlas_api::dtos::boards_tasks::ReferenceDto {
        atlas_api::dtos::boards_tasks::ReferenceDto {
            id: Uuid::now_v7(),
            kind: "relates".to_owned(),
            target_task_id: Some(Uuid::now_v7()),
            target_readable_id: Some("ATL-10".to_owned()),
            target_document_id: None,
            target_title: None,
            target_resolved: true,
            created_by: make_actor_dto(),
            created_at: Utc::now(),
        }
    }

    fn make_task_backlink_dto() -> atlas_api::dtos::boards_tasks::TaskBacklinkDto {
        atlas_api::dtos::boards_tasks::TaskBacklinkDto {
            source_task_id: Uuid::now_v7(),
            source_readable_id: "ATL-5".to_owned(),
            source_title: "Blocker task".to_owned(),
            kind: "blocks".to_owned(),
        }
    }

    fn make_assignee_dto() -> atlas_api::dtos::boards_tasks::AssigneeDto {
        atlas_api::dtos::boards_tasks::AssigneeDto {
            assignee: make_actor_dto(),
            assigned_by: make_actor_dto(),
            assigned_at: Utc::now(),
        }
    }

    fn make_checklist_item_dto(promoted: bool) -> atlas_api::dtos::boards_tasks::ChecklistItemDto {
        atlas_api::dtos::boards_tasks::ChecklistItemDto {
            id: Uuid::now_v7(),
            task_id: Uuid::now_v7(),
            title: "Write docs".to_owned(),
            checked: false,
            position_key: "a0".to_owned(),
            promoted_task_id: if promoted { Some(Uuid::now_v7()) } else { None },
            promoted_readable_id: if promoted {
                Some("ATL-99".to_owned())
            } else {
                None
            },
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_activity_entry_dto(
        task_readable_id: &str,
    ) -> atlas_api::dtos::boards_tasks::ActivityEntryDto {
        atlas_api::dtos::boards_tasks::ActivityEntryDto {
            id: Uuid::now_v7(),
            kind: "moved".to_owned(),
            actor: make_actor_dto(),
            payload: serde_json::json!({"from": "Todo", "to": "In Progress"}),
            created_at: Utc::now(),
            task_id: Uuid::now_v7(),
            task_readable_id: task_readable_id.to_owned(),
        }
    }

    fn make_revision_meta_dto(with_actor: bool) -> atlas_api::dtos::documents::RevisionMetaDto {
        atlas_api::dtos::documents::RevisionMetaDto {
            id: Uuid::now_v7(),
            seq: 3,
            is_anchor: false,
            actor: if with_actor {
                Some(make_actor_dto())
            } else {
                None
            },
            created_at: Utc::now(),
        }
    }

    fn make_revision_content_dto(
        with_actor: bool,
    ) -> atlas_api::dtos::documents::RevisionContentDto {
        atlas_api::dtos::documents::RevisionContentDto {
            id: Uuid::now_v7(),
            seq: 3,
            content: "# Hello\nWorld".to_owned(),
            actor: if with_actor {
                Some(make_actor_dto())
            } else {
                None
            },
            created_at: Utc::now(),
        }
    }

    fn make_backlink_dto(slug: Option<&str>) -> atlas_api::dtos::documents::BacklinkDto {
        atlas_api::dtos::documents::BacklinkDto {
            source_document_id: Uuid::now_v7(),
            source_slug: slug.map(str::to_owned),
            source_title: "Source Doc".to_owned(),
            display_title: "Source Doc".to_owned(),
        }
    }

    #[test]
    fn task_ref_projection_contract_fields() {
        let dto = make_reference_dto();
        let proj = TaskRefProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(
            &value,
            &[
                "manual_reference_id",
                "manual_kind",
                "origins",
                "target_resolved",
            ],
            &["target_readable_id", "target_document_id", "target_title"],
        );
    }

    #[test]
    fn unified_task_ref_projection_preserves_manual_reference_actionability() {
        let manual_reference_id = Uuid::now_v7();
        let dto = atlas_api::dtos::boards_tasks::UnifiedReferenceDto {
            id: manual_reference_id,
            origins: vec![
                atlas_api::dtos::boards_tasks::ReferenceOriginDto::Manual,
                atlas_api::dtos::boards_tasks::ReferenceOriginDto::Wikilink,
            ],
            manual_reference_id: Some(manual_reference_id),
            manual_kind: Some("relates".to_owned()),
            target_task_id: None,
            target_readable_id: None,
            target_document_id: Some(Uuid::now_v7()),
            target_title: Some("Linked document".to_owned()),
            target_resolved: true,
            manual_created_by: Some(make_actor_dto()),
            manual_created_at: Some(Utc::now()),
        };

        let proj = TaskRefProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();

        assert_eq!(proj.manual_reference_id, Some(manual_reference_id));
        assert_eq!(
            value["manual_reference_id"],
            manual_reference_id.to_string()
        );
        assert_eq!(proj.row()[0], manual_reference_id.to_string());
    }

    #[test]
    fn task_ref_projection_preserves_manual_id_and_drops_attribution() {
        let dto = make_reference_dto();
        let reference_id = dto.id;
        let proj = TaskRefProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["manual_reference_id"], reference_id.to_string());
        assert!(
            value.get("created_by").is_none(),
            "created_by must be dropped"
        );
        assert!(
            value.get("created_at").is_none(),
            "created_at must be dropped"
        );
    }

    #[test]
    fn task_ref_projection_optional_targets_absent_when_none() {
        let dto = atlas_api::dtos::boards_tasks::ReferenceDto {
            id: Uuid::now_v7(),
            kind: "relates".to_owned(),
            target_task_id: None,
            target_readable_id: None,
            target_document_id: None,
            target_title: None,
            target_resolved: false,
            created_by: make_actor_dto(),
            created_at: Utc::now(),
        };
        let proj = TaskRefProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(value.get("target_readable_id").is_none());
        assert!(value.get("target_document_id").is_none());
        assert!(value.get("target_title").is_none());
    }

    #[test]
    fn task_backlink_projection_contract_fields() {
        let dto = make_task_backlink_dto();
        let proj = TaskBacklinkProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(&value, &["source_readable_id", "source_title", "kind"], &[]);
    }

    #[test]
    fn task_backlink_projection_drops_source_task_id() {
        let dto = make_task_backlink_dto();
        let proj = TaskBacklinkProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(
            value.get("source_task_id").is_none(),
            "source_task_id must be dropped"
        );
    }

    #[test]
    fn task_assignee_projection_contract_fields() {
        let dto = make_assignee_dto();
        let proj = TaskAssigneeProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(&value, &["type", "assigned_at"], &["display_name"]);
    }

    #[test]
    fn task_assignee_projection_drops_assigned_by() {
        let dto = make_assignee_dto();
        let proj = TaskAssigneeProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(
            value.get("assigned_by").is_none(),
            "assigned_by must be dropped"
        );
    }

    #[test]
    fn checklist_item_projection_contract_fields() {
        let dto = make_checklist_item_dto(false);
        let proj = ChecklistItemProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(
            &value,
            &["id", "title", "checked"],
            &["promoted_readable_id"],
        );
    }

    #[test]
    fn checklist_item_promoted_readable_id_absent_when_none() {
        let dto = make_checklist_item_dto(false);
        let proj = ChecklistItemProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(value.get("promoted_readable_id").is_none());
    }

    #[test]
    fn checklist_item_promoted_readable_id_present_when_promoted() {
        let dto = make_checklist_item_dto(true);
        let proj = ChecklistItemProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["promoted_readable_id"], "ATL-99");
    }

    #[test]
    fn task_activity_projection_contract_fields() {
        let dto = make_activity_entry_dto("ATL-1");
        let proj = TaskActivityProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(&value, &["kind", "actor", "payload", "created_at"], &[]);
    }

    #[test]
    fn task_activity_projection_drops_id_and_task_id() {
        let dto = make_activity_entry_dto("ATL-1");
        let proj = TaskActivityProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(value.get("id").is_none(), "id must be dropped");
        assert!(value.get("task_id").is_none(), "task_id must be dropped");
        assert!(
            value.get("task_readable_id").is_none(),
            "task_readable_id must be dropped"
        );
    }

    #[test]
    fn subtask_projection_is_task_summary_with_assignees() {
        let dto = make_task_summary_dto();
        let proj = SubtaskProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(
            value.get("assignees").is_some(),
            "SubtaskProjection must include assignees field"
        );
        assert_projection_fields(
            &value,
            &[
                "readable_id",
                "title",
                "board_name",
                "column_name",
                "labels",
                "assignees",
                "updated_at",
            ],
            &["priority", "estimate"],
        );
    }

    #[test]
    fn doc_history_projection_contract_fields() {
        let dto = make_revision_meta_dto(true);
        let proj = DocHistoryProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(&value, &["seq", "is_anchor", "created_at"], &["actor"]);
    }

    #[test]
    fn doc_history_projection_actor_absent_when_none() {
        let dto = make_revision_meta_dto(false);
        let proj = DocHistoryProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(
            value.get("actor").is_none(),
            "actor must be absent when None"
        );
    }

    #[test]
    fn doc_history_projection_drops_id() {
        let dto = make_revision_meta_dto(false);
        let proj = DocHistoryProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(value.get("id").is_none(), "id must be dropped");
    }

    #[test]
    fn doc_revision_projection_contract_fields() {
        let dto = make_revision_content_dto(true);
        let proj = DocRevisionProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(&value, &["seq", "content", "created_at"], &["actor"]);
    }

    #[test]
    fn doc_revision_projection_drops_id() {
        let dto = make_revision_content_dto(false);
        let proj = DocRevisionProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(value.get("id").is_none(), "id must be dropped");
    }

    #[test]
    fn doc_backlink_projection_contract_fields() {
        let dto = make_backlink_dto(Some("my-note"));
        let proj = DocBacklinkProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(&value, &["source_title", "display_title"], &["source_slug"]);
    }

    #[test]
    fn doc_backlink_projection_slug_absent_when_none() {
        let dto = make_backlink_dto(None);
        let proj = DocBacklinkProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(value.get("source_slug").is_none(), "slug absent when None");
    }

    #[test]
    fn doc_backlink_projection_drops_source_document_id() {
        let dto = make_backlink_dto(None);
        let proj = DocBacklinkProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(
            value.get("source_document_id").is_none(),
            "source_document_id must be dropped"
        );
    }

    #[test]
    fn workspace_activity_projection_contract_fields() {
        let dto = make_activity_entry_dto("ATL-42");
        let proj = WorkspaceActivityProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(
            &value,
            &["task_readable_id", "kind", "actor", "payload", "created_at"],
            &[],
        );
    }

    #[test]
    fn workspace_activity_projection_includes_task_readable_id() {
        let dto = make_activity_entry_dto("ATL-42");
        let proj = WorkspaceActivityProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["task_readable_id"], "ATL-42");
    }

    #[test]
    fn workspace_activity_projection_drops_id_and_task_id() {
        let dto = make_activity_entry_dto("ATL-42");
        let proj = WorkspaceActivityProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(value.get("id").is_none());
        assert!(value.get("task_id").is_none());
    }

    // -----------------------------------------------------------------------
    // Admin projection contract tests (Batch 5-A)
    // -----------------------------------------------------------------------

    fn make_user_dto() -> UserDto {
        UserDto {
            id: Uuid::now_v7(),
            username: "alice".to_owned(),
            display_name: "Alice".to_owned(),
            email: Some("alice@example.com".to_owned()),
            is_root: false,
            is_system_admin: false,
            disabled_at: None,
            activated_at: Some(Utc::now()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn user_projection_contract_fields() {
        let proj = UserProjection::from(make_user_dto());
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(
            &value,
            &[
                "id",
                "username",
                "display_name",
                "is_root",
                "is_system_admin",
                "created_at",
                "updated_at",
            ],
            &["email", "disabled_at", "activated_at"],
        );
    }

    #[test]
    fn user_projection_optional_fields_absent_when_none() {
        let mut dto = make_user_dto();
        dto.email = None;
        dto.disabled_at = None;
        dto.activated_at = None;
        let proj = UserProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert!(value.get("email").is_none());
        assert!(value.get("disabled_at").is_none());
        assert!(value.get("activated_at").is_none());
    }

    #[test]
    fn user_created_projection_surfaces_activation_link() {
        use atlas_api::dtos::CreateUserResponse;
        let resp = CreateUserResponse {
            user: make_user_dto(),
            activation_link: "/activate/tok123".to_owned(),
        };
        let proj = UserCreatedProjection::from(resp);
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["activation_link"], "/activate/tok123");
        assert_projection_fields(
            &value,
            &[
                "id",
                "username",
                "display_name",
                "is_root",
                "is_system_admin",
                "created_at",
                "updated_at",
                "activation_link",
            ],
            &["email"],
        );
    }

    #[test]
    fn activation_link_projection_contract() {
        use atlas_api::dtos::ActivationLinkResponse;
        let resp = ActivationLinkResponse {
            activation_link: "/activate/abc".to_owned(),
        };
        let proj = ActivationLinkProjection::from(resp);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(&value, &["activation_link"], &[]);
        assert_eq!(value["activation_link"], "/activate/abc");
    }

    #[test]
    fn user_membership_projection_contract() {
        use atlas_api::dtos::UserMembershipDto;
        let dto = UserMembershipDto {
            workspace_slug: "my-ws".to_owned(),
            workspace_name: "My Workspace".to_owned(),
            role: "member".to_owned(),
        };
        let proj = UserMembershipProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(&value, &["workspace_slug", "workspace_name", "role"], &[]);
    }

    fn make_api_key_dto() -> ApiKeyDto {
        ApiKeyDto {
            id: Uuid::now_v7(),
            name: "test-key".to_owned(),
            r#type: "agent".to_owned(),
            expires_at: None,
            last_used_at: None,
            revoked_at: None,
            created_at: Utc::now(),
            is_global: false,
            scopes: vec![],
        }
    }

    #[test]
    fn api_key_projection_contract_fields() {
        let proj = ApiKeyProjection::from(make_api_key_dto());
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(
            &value,
            &["id", "name", "type", "created_at", "is_global", "scopes"],
            &["expires_at", "last_used_at", "revoked_at"],
        );
    }

    #[test]
    fn api_key_projection_renders_scopes_as_wire_strings() {
        let mut dto = make_api_key_dto();
        dto.scopes = vec![ApiKeyScope::TasksRead, ApiKeyScope::ProjectsDelete];
        let proj = ApiKeyProjection::from(dto);

        assert_eq!(proj.scopes, vec!["tasks:read", "projects:delete"]);

        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["scopes"][0], "tasks:read");
        assert_eq!(value["scopes"][1], "projects:delete");
    }

    #[test]
    fn api_key_projection_does_not_include_secret() {
        let proj = ApiKeyProjection::from(make_api_key_dto());
        let value = serde_json::to_value(&proj).unwrap();
        assert!(
            value.get("secret").is_none(),
            "list projection must not expose secret"
        );
    }

    #[test]
    fn api_key_created_projection_surfaces_secret() {
        use atlas_api::dtos::ApiKeyCreated;
        let dto = ApiKeyCreated {
            id: Uuid::now_v7(),
            name: "new-key".to_owned(),
            secret: "atlas_supersecret".to_owned(),
            r#type: "cli".to_owned(),
            expires_at: None,
            created_at: Utc::now(),
            scopes: vec![],
        };
        let proj = ApiKeyCreatedProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(
            &value,
            &["id", "name", "secret", "type", "created_at"],
            &["expires_at"],
        );
        assert_eq!(value["secret"], "atlas_supersecret");
    }

    fn make_api_key_grant_dto() -> ApiKeyGrantDto {
        use atlas_api::dtos::GrantedByDto;
        ApiKeyGrantDto {
            id: Uuid::now_v7(),
            role: "editor".to_owned(),
            resource_kind: "workspace".to_owned(),
            resource_label: "My Workspace".to_owned(),
            workspace_slug: "my-ws".to_owned(),
            project_slug: None,
            granted_by: Some(GrantedByDto {
                id: Uuid::now_v7(),
                display: "Alice".to_owned(),
                principal_type: "user".to_owned(),
            }),
        }
    }

    #[test]
    fn api_key_grant_projection_contract_fields() {
        let proj = ApiKeyGrantProjection::from(make_api_key_grant_dto());
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(
            &value,
            &[
                "id",
                "role",
                "resource_kind",
                "resource_label",
                "workspace_slug",
            ],
            &["project_slug", "granted_by"],
        );
    }

    fn make_group_dto() -> GroupDto {
        GroupDto {
            id: Uuid::now_v7(),
            workspace_id: Uuid::now_v7(),
            name: "devs".to_owned(),
            created_by: Uuid::now_v7(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn group_projection_contract_fields() {
        let proj = GroupProjection::from(make_group_dto());
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(&value, &["id", "name", "created_at", "updated_at"], &[]);
    }

    #[test]
    fn group_projection_drops_workspace_id_and_created_by() {
        let proj = GroupProjection::from(make_group_dto());
        let value = serde_json::to_value(&proj).unwrap();
        assert!(value.get("workspace_id").is_none());
        assert!(value.get("created_by").is_none());
    }

    #[test]
    fn group_member_projection_contract_fields() {
        use atlas_api::dtos::groups::GroupMemberDto;
        let dto = GroupMemberDto {
            group_id: Uuid::now_v7(),
            user_id: Uuid::now_v7(),
            created_at: Utc::now(),
        };
        let proj = GroupMemberProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(&value, &["group_id", "user_id", "created_at"], &[]);
    }

    #[test]
    fn delete_by_id_projection_contract() {
        let id = Uuid::now_v7();
        let proj = DeleteByIdProjection { deleted: true, id };
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(&value, &["deleted", "id"], &[]);
        assert_eq!(value["deleted"], true);
    }

    // -----------------------------------------------------------------------
    // Batch 5-B projections
    // -----------------------------------------------------------------------

    fn make_status_template_dto(color: Option<&str>) -> StatusTemplateDto {
        StatusTemplateDto {
            id: Uuid::now_v7(),
            workspace_id: Uuid::now_v7(),
            name: "In Progress".to_owned(),
            color: color.map(str::to_owned),
            position_key: "a0".to_owned(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn status_template_projection_required_fields() {
        let proj = StatusTemplateProjection::from(make_status_template_dto(None));
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(
            &value,
            &["id", "name", "position_key", "updated_at"],
            &["color"],
        );
    }

    #[test]
    fn status_template_projection_drops_workspace_id_and_created_at() {
        let proj = StatusTemplateProjection::from(make_status_template_dto(None));
        let value = serde_json::to_value(&proj).unwrap();
        assert!(value.get("workspace_id").is_none());
        assert!(value.get("created_at").is_none());
    }

    #[test]
    fn status_template_projection_includes_color_when_present() {
        let proj = StatusTemplateProjection::from(make_status_template_dto(Some("blue")));
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["color"], "blue");
    }

    #[test]
    fn status_template_projection_omits_color_when_absent() {
        let proj = StatusTemplateProjection::from(make_status_template_dto(None));
        let value = serde_json::to_value(&proj).unwrap();
        assert!(value.get("color").is_none());
    }

    fn make_saved_search_dto() -> SavedSearchDto {
        SavedSearchDto {
            id: Uuid::now_v7(),
            workspace_id: Uuid::now_v7(),
            name: "Open bugs".to_owned(),
            query: "status:open tag:bug".to_owned(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn saved_search_projection_contract_fields() {
        let proj = SavedSearchProjection::from(make_saved_search_dto());
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(&value, &["id", "name", "query"], &[]);
    }

    #[test]
    fn saved_search_projection_drops_workspace_id_and_timestamps() {
        let proj = SavedSearchProjection::from(make_saved_search_dto());
        let value = serde_json::to_value(&proj).unwrap();
        assert!(value.get("workspace_id").is_none());
        assert!(value.get("created_at").is_none());
        assert!(value.get("updated_at").is_none());
    }

    #[test]
    fn saved_search_projection_preserves_query() {
        let proj = SavedSearchProjection::from(make_saved_search_dto());
        assert_eq!(proj.query, "status:open tag:bug");
    }

    fn make_task_view_dto(filters: TaskViewFiltersDto) -> TaskViewDto {
        TaskViewDto {
            id: Uuid::now_v7(),
            workspace_id: Uuid::now_v7(),
            name: "High priority".to_owned(),
            filters,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn task_view_projection_contract_fields() {
        let proj = TaskViewProjection::from(make_task_view_dto(TaskViewFiltersDto::default()));
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(&value, &["id", "name", "filters"], &[]);
    }

    #[test]
    fn task_view_projection_drops_workspace_id_and_timestamps() {
        let proj = TaskViewProjection::from(make_task_view_dto(TaskViewFiltersDto::default()));
        let value = serde_json::to_value(&proj).unwrap();
        assert!(value.get("workspace_id").is_none());
        assert!(value.get("created_at").is_none());
        assert!(value.get("updated_at").is_none());
    }

    #[test]
    fn task_view_projection_filters_empty_fields_omitted() {
        let proj = TaskViewProjection::from(make_task_view_dto(TaskViewFiltersDto::default()));
        let value = serde_json::to_value(&proj).unwrap();
        let filters = value["filters"].as_object().unwrap();
        assert!(
            filters.is_empty(),
            "empty default filters must serialize as {{}}"
        );
    }

    #[test]
    fn task_view_projection_includes_non_empty_filter_fields() {
        let filters = TaskViewFiltersDto {
            sort: Some("priority_desc".to_owned()),
            priorities: vec!["high".to_owned(), "urgent".to_owned()],
            ..Default::default()
        };
        let proj = TaskViewProjection::from(make_task_view_dto(filters));
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["filters"]["sort"], "priority_desc");
        let prios = value["filters"]["priorities"].as_array().unwrap();
        assert_eq!(prios.len(), 2);
    }

    fn make_property_definition_dto() -> PropertyDefinitionDto {
        PropertyDefinitionDto {
            id: Uuid::now_v7(),
            key: "due_date".to_owned(),
            name: "Due Date".to_owned(),
            kind: "date".to_owned(),
            options: None,
            applies_to: "task".to_owned(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn property_definition_projection_contract_fields() {
        let proj = PropertyDefinitionProjection::from(make_property_definition_dto());
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(
            &value,
            &["id", "key", "name", "kind", "applies_to", "created_at"],
            &["options"],
        );
    }

    #[test]
    fn property_definition_projection_omits_options_when_absent() {
        let proj = PropertyDefinitionProjection::from(make_property_definition_dto());
        let value = serde_json::to_value(&proj).unwrap();
        assert!(value.get("options").is_none());
    }

    #[test]
    fn property_definition_projection_includes_options_for_select_kind() {
        use serde_json::json;
        let mut dto = make_property_definition_dto();
        dto.kind = "select".to_owned();
        dto.options = Some(json!(["todo", "in_progress", "done"]));
        let proj = PropertyDefinitionProjection::from(dto);
        let value = serde_json::to_value(&proj).unwrap();
        let opts = value["options"].as_array().unwrap();
        assert_eq!(opts.len(), 3);
    }

    // -----------------------------------------------------------------------
    // Grant projections (WU-29)
    // -----------------------------------------------------------------------

    fn make_grant_dto() -> atlas_api::dtos::GrantDto {
        atlas_api::dtos::GrantDto {
            id: Uuid::now_v7(),
            principal: atlas_api::dtos::GrantPrincipal {
                r#type: "user".to_owned(),
                id: Uuid::now_v7(),
            },
            role: "viewer".to_owned(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn grant_projection_contract_fields() {
        let proj = GrantProjection::from(make_grant_dto());
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(&value, &["id", "principal", "role", "created_at"], &[]);
    }

    #[test]
    fn grant_projection_principal_type_serializes_as_type_key() {
        let proj = GrantProjection::from(make_grant_dto());
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["principal"]["type"], "user");
        assert!(value["principal"].get("id").is_some());
    }

    #[test]
    fn grant_projection_role_preserved() {
        let proj = GrantProjection::from(make_grant_dto());
        assert_eq!(proj.role, "viewer");
    }

    // -----------------------------------------------------------------------
    // Audit entry projections (WU-29)
    // -----------------------------------------------------------------------

    fn make_audit_entry_dto(
        actor_type: &str,
        display_name: Option<&str>,
        target_id: Option<Uuid>,
        target_label: Option<&str>,
    ) -> atlas_api::dtos::audit::AuditEntryDto {
        atlas_api::dtos::audit::AuditEntryDto {
            id: Uuid::now_v7(),
            workspace_id: Some(Uuid::now_v7()),
            actor: atlas_api::dtos::documents::ActorDto {
                r#type: actor_type.to_owned(),
                id: Uuid::now_v7(),
                display_name: display_name.map(str::to_owned),
                key_type: if actor_type == "api_key" {
                    Some("agent".to_owned())
                } else {
                    None
                },
                account_status: if actor_type == "user" {
                    Some("active".to_owned())
                } else {
                    None
                },
            },
            action: "membership.role_changed".to_owned(),
            target_type: "user".to_owned(),
            target_id,
            target_label: target_label.map(str::to_owned),
            metadata: serde_json::json!({"old_role": "member", "new_role": "admin"}),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn audit_entry_projection_drops_id_and_workspace_id() {
        let proj =
            AuditEntryProjection::from(make_audit_entry_dto("user", Some("Alice"), None, None));
        let value = serde_json::to_value(&proj).unwrap();
        assert!(value.get("id").is_none(), "id must be dropped");
        assert!(
            value.get("workspace_id").is_none(),
            "workspace_id must be dropped"
        );
    }

    #[test]
    fn audit_entry_projection_contract_required_fields() {
        let proj =
            AuditEntryProjection::from(make_audit_entry_dto("user", Some("Alice"), None, None));
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(
            &value,
            &["actor", "action", "target_type", "metadata", "created_at"],
            &["target_id", "target_label"],
        );
    }

    #[test]
    fn audit_entry_projection_actor_type_and_display_name() {
        let proj =
            AuditEntryProjection::from(make_audit_entry_dto("user", Some("Alice"), None, None));
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["actor"]["type"], "user");
        assert_eq!(value["actor"]["display_name"], "Alice");
        assert_eq!(value["actor"]["account_status"], "active");
        assert!(
            value["actor"].get("id").is_none(),
            "actor.id must be dropped"
        );
    }

    #[test]
    fn audit_entry_projection_api_key_actor_surfaces_key_type() {
        let proj =
            AuditEntryProjection::from(make_audit_entry_dto("api_key", Some("ci-bot"), None, None));
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["actor"]["type"], "api_key");
        assert_eq!(value["actor"]["key_type"], "agent");
        assert!(
            value["actor"].get("account_status").is_none(),
            "account_status absent for api_key actor"
        );
    }

    #[test]
    fn audit_entry_projection_target_id_absent_when_none() {
        let proj =
            AuditEntryProjection::from(make_audit_entry_dto("user", Some("Alice"), None, None));
        let value = serde_json::to_value(&proj).unwrap();
        assert!(value.get("target_id").is_none());
    }

    #[test]
    fn audit_entry_projection_target_id_present_when_some() {
        let tid = Uuid::now_v7();
        let proj = AuditEntryProjection::from(make_audit_entry_dto(
            "user",
            Some("Alice"),
            Some(tid),
            None,
        ));
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["target_id"].as_str().unwrap(), tid.to_string());
    }

    #[test]
    fn audit_entry_projection_target_label_present_when_some() {
        let proj = AuditEntryProjection::from(make_audit_entry_dto(
            "user",
            Some("Alice"),
            Some(Uuid::now_v7()),
            Some("bob"),
        ));
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["target_label"], "bob");
    }

    #[test]
    fn audit_entry_projection_metadata_passthrough() {
        let proj =
            AuditEntryProjection::from(make_audit_entry_dto("user", Some("Alice"), None, None));
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["metadata"]["old_role"], "member");
        assert_eq!(value["metadata"]["new_role"], "admin");
    }

    // -----------------------------------------------------------------------
    // AttachProjection (WU-34)
    // -----------------------------------------------------------------------

    fn make_attachment_dto() -> atlas_api::dtos::documents::AttachmentDto {
        atlas_api::dtos::documents::AttachmentDto {
            id: Uuid::now_v7(),
            document_id: Uuid::now_v7(),
            file_name: "report.pdf".to_owned(),
            content_type: "application/pdf".to_owned(),
            size_bytes: 2048,
            sha256: "deadbeef".to_owned(),
            actor: None,
            created_at: Utc::now(),
        }
    }

    fn make_task_attachment_dto() -> atlas_api::dtos::boards_tasks::TaskAttachmentDto {
        atlas_api::dtos::boards_tasks::TaskAttachmentDto {
            id: Uuid::now_v7(),
            file_name: "screenshot.png".to_owned(),
            content_type: "image/png".to_owned(),
            size_bytes: 512,
            created_by: make_actor_dto(), // reuses the existing helper in this test module
            created_at: Utc::now(),
        }
    }

    #[test]
    fn attach_projection_contract_fields() {
        let proj = AttachProjection::from(make_attachment_dto());
        let value = serde_json::to_value(&proj).unwrap();
        assert_projection_fields(
            &value,
            &["id", "filename", "content_type", "size", "created_at"],
            &[],
        );
    }

    #[test]
    fn attach_projection_from_doc_attachment_drops_internal_fields() {
        let proj = AttachProjection::from(make_attachment_dto());
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["filename"], "report.pdf");
        assert_eq!(value["size"], 2048);
        assert!(
            value.get("document_id").is_none(),
            "document_id must be dropped"
        );
        assert!(value.get("sha256").is_none(), "sha256 must be dropped");
        assert!(
            value.get("file_name").is_none(),
            "raw file_name must be absent"
        );
        assert!(
            value.get("size_bytes").is_none(),
            "raw size_bytes must be absent"
        );
    }

    #[test]
    fn attach_projection_from_task_attachment_drops_created_by() {
        let proj = AttachProjection::from(make_task_attachment_dto());
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["filename"], "screenshot.png");
        assert_eq!(value["size"], 512);
        assert!(
            value.get("created_by").is_none(),
            "created_by must be dropped"
        );
        assert!(
            value.get("size_bytes").is_none(),
            "raw size_bytes must be absent"
        );
    }
}
