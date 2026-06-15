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

    fn task_hit(readable_id: &str) -> SearchHitDto {
        SearchHitDto {
            id: Uuid::now_v7(),
            kind: SearchKindDto::Task,
            readable_id: Some(readable_id.to_string()),
            title: "Test".to_string(),
            snippet: None,
            score: 0.9,
            updated_at: now(),
            project_slug: None,
        }
    }

    fn doc_hit() -> SearchHitDto {
        SearchHitDto {
            id: Uuid::now_v7(),
            kind: SearchKindDto::Document,
            readable_id: None,
            title: "Test".to_string(),
            snippet: None,
            score: 0.9,
            updated_at: now(),
            project_slug: None,
        }
    }

    #[test]
    fn task_hit_serializes_readable_id() {
        let dto = task_hit("ATL-1");
        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(json["readable_id"], "ATL-1");
    }

    #[test]
    fn task_hit_omits_readable_id_when_none() {
        let dto = doc_hit();
        let json = serde_json::to_value(&dto).unwrap();
        assert!(
            json.get("readable_id").is_none(),
            "readable_id must be absent when None"
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
        let dto = doc_hit();
        let json = serde_json::to_value(&dto).unwrap();
        assert!(json.get("snippet").is_none());
    }

    #[test]
    fn snippet_present_when_some() {
        let mut dto = task_hit("ATL-2");
        dto.snippet = Some("<mark>highlighted</mark> text".to_string());
        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(json["snippet"], "<mark>highlighted</mark> text");
    }

    #[test]
    fn project_slug_absent_when_none() {
        let dto = doc_hit();
        let json = serde_json::to_value(&dto).unwrap();
        assert!(json.get("project_slug").is_none());
    }

    #[test]
    fn project_slug_present_when_some() {
        let mut dto = doc_hit();
        dto.project_slug = Some("my-project".to_string());
        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(json["project_slug"], "my-project");
    }

    #[test]
    fn no_slug_field_in_wire_shape() {
        let dto = doc_hit();
        let json = serde_json::to_value(&dto).unwrap();
        assert!(
            json.get("slug").is_none(),
            "bare 'slug' must not appear in the wire shape; spec uses 'project_slug'"
        );
    }
}
