use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Request body for `POST /v1/workspaces/{ws}/projects/{ps}/documents`.
///
/// The slug is always server-generated; any slug field provided by the client
/// is silently ignored.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateDocumentRequest {
    pub title: String,
    pub folder_id: Option<uuid::Uuid>,
    pub content: Option<String>,
}

/// Request body for `PATCH /v1/workspaces/{ws}/documents/{slug}`.
///
/// Updates document metadata (title, folder). Use `PUT .../content` to update
/// content with CAS.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UpdateDocumentRequest {
    pub title: Option<String>,
    pub folder_id: Option<uuid::Uuid>,
}

/// Request body for `PUT /v1/workspaces/{ws}/documents/{slug}/content`.
///
/// Uses compare-and-swap semantics: `base_revision_id` must match the
/// document's current revision or the server responds with 409 and a
/// `ConflictProblemDto` body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UpdateContentRequest {
    pub content: String,
    pub base_revision_id: uuid::Uuid,
}

/// Request body for `PATCH /v1/workspaces/{ws}/documents/{slug}/move`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct MoveDocumentRequest {
    pub folder_id: Option<uuid::Uuid>,
}

/// Request body for `POST /v1/workspaces/{ws}/documents/{slug}/copy`.
///
/// `folder_id` is the destination folder for the copy. When omitted, the copy
/// lands in the same folder as the source document.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CopyDocumentRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder_id: Option<uuid::Uuid>,
}

/// Actor attribution attached to revisions and attachments.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ActorDto {
    pub r#type: String,
    pub id: uuid::Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// Full document representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct DocumentDto {
    pub id: uuid::Uuid,
    pub workspace_id: uuid::Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<uuid::Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder_id: Option<uuid::Uuid>,
    pub slug: Option<String>,
    pub title: String,
    pub content: String,
    pub head_revision_id: uuid::Uuid,
    pub head_seq: i64,
    pub frontmatter: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Lightweight document summary for list endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct DocumentSummaryDto {
    pub id: uuid::Uuid,
    pub slug: Option<String>,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder_id: Option<uuid::Uuid>,
    pub head_seq: i64,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Revision metadata returned by the history endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct RevisionMetaDto {
    pub id: uuid::Uuid,
    pub seq: i64,
    pub is_anchor: bool,
    pub actor: Option<ActorDto>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Full revision content at a specific sequence number.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct RevisionContentDto {
    pub id: uuid::Uuid,
    pub seq: i64,
    pub content: String,
    pub actor: Option<ActorDto>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// A document that links to this document (backlink).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct BacklinkDto {
    pub source_document_id: uuid::Uuid,
    pub source_slug: Option<String>,
    pub source_title: String,
    pub display_title: String,
}

/// Document frontmatter extracted from the leading YAML block.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct FrontmatterDto {
    pub data: serde_json::Value,
}

/// Attachment metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct AttachmentDto {
    pub id: uuid::Uuid,
    pub document_id: uuid::Uuid,
    pub file_name: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub actor: Option<ActorDto>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// RFC 9457 problem+json extension for CAS revision conflicts (status 409).
///
/// Flattens `ProblemDetails` fields alongside conflict-specific fields so the
/// client receives a single `application/problem+json` body.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ConflictProblemDto {
    pub r#type: String,
    pub title: String,
    pub status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// The revision ID that is currently head (the client's token is stale).
    pub current_revision_id: uuid::Uuid,
    /// Sequence number of the current head revision.
    pub current_seq: i64,
    /// Unified diff from the client's base revision to the current head.
    pub base_to_current_patch: String,
}

impl ConflictProblemDto {
    pub fn new(
        current_revision_id: uuid::Uuid,
        current_seq: i64,
        base_to_current_patch: String,
    ) -> Self {
        Self {
            r#type: "urn:atlas:error:revision-conflict".into(),
            title: "Revision Conflict".into(),
            status: 409,
            detail: Some(
                "The base_revision_id does not match the current revision. \
                 Apply base_to_current_patch and retry."
                    .into(),
            ),
            instance: None,
            request_id: None,
            hint: Some(
                "Apply the provided patch to your local content, then retry with the new revision id."
                    .into(),
            ),
            current_revision_id,
            current_seq,
            base_to_current_patch,
        }
    }
}
