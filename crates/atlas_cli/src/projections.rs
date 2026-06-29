#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use atlas_api::dtos::search::{SearchHitDto, SearchKindDto};
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
}
