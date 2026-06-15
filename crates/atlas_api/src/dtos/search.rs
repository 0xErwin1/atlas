use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Discriminant indicating whether a search hit is a document or a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub enum SearchKindDto {
    Document,
    Task,
}

/// A single search result item returned by `GET /v1/workspaces/{ws}/search`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SearchHitDto {
    pub id: Uuid,
    pub kind: SearchKindDto,
    /// Task readable ID (e.g. `"ATL-42"`). Present only when `kind = task`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readable_id: Option<String>,
    /// Document slug. Present only when `kind = document`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    pub title: String,
    /// Highlighted snippet with `<mark>…</mark>` markers.
    /// Absent for title-only matches and filter-only queries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    pub score: f32,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Owning project slug; absent for workspace-root documents with no project.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_slug: Option<String>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn now() -> chrono::DateTime<Utc> {
        Utc::now()
    }

    fn hit(kind: SearchKindDto, readable_id: Option<&str>, slug: Option<&str>) -> SearchHitDto {
        SearchHitDto {
            id: Uuid::now_v7(),
            kind,
            readable_id: readable_id.map(|s| s.to_string()),
            slug: slug.map(|s| s.to_string()),
            title: "Test".to_string(),
            snippet: None,
            score: 0.9,
            updated_at: now(),
            project_slug: None,
        }
    }

    #[test]
    fn task_hit_serializes_readable_id_omits_slug() {
        let dto = hit(SearchKindDto::Task, Some("ATL-1"), None);
        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(json["readable_id"], "ATL-1");
        assert!(json.get("slug").is_none(), "slug must be absent for tasks");
    }

    #[test]
    fn document_hit_serializes_slug_omits_readable_id() {
        let dto = hit(SearchKindDto::Document, None, Some("my-doc"));
        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(json["slug"], "my-doc");
        assert!(
            json.get("readable_id").is_none(),
            "readable_id must be absent for documents"
        );
    }

    #[test]
    fn document_kind_serializes_as_document_string() {
        let json = serde_json::to_value(SearchKindDto::Document).unwrap();
        assert_eq!(json, "document");
    }

    #[test]
    fn task_kind_serializes_as_task_string() {
        let json = serde_json::to_value(SearchKindDto::Task).unwrap();
        assert_eq!(json, "task");
    }

    #[test]
    fn snippet_absent_when_none() {
        let dto = hit(SearchKindDto::Document, None, Some("slug"));
        let json = serde_json::to_value(&dto).unwrap();
        assert!(json.get("snippet").is_none());
    }

    #[test]
    fn snippet_present_when_some() {
        let mut dto = hit(SearchKindDto::Task, Some("ATL-2"), None);
        dto.snippet = Some("<mark>highlighted</mark> text".to_string());
        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(json["snippet"], "<mark>highlighted</mark> text");
    }
}
