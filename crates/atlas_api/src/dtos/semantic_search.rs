use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

const SEMANTIC_CURSOR_BYTES: usize = 21;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub enum SemanticSearchKindDto {
    Document,
    Task,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub enum SemanticSearchSourceDto {
    Title,
    Content,
    Comment,
    AttachmentName,
    Checklist,
    Subtask,
    Aggregate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SemanticSearchHitDto {
    pub id: Uuid,
    pub kind: SemanticSearchKindDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub readable_id: Option<String>,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column_name: Option<String>,
    pub similarity: f32,
    pub source: SemanticSearchSourceDto,
    pub excerpt: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SemanticSearchCursor {
    pub similarity: f32,
    pub kind: SemanticSearchKindDto,
    pub id: Uuid,
}

impl SemanticSearchCursor {
    pub fn encode(&self) -> String {
        let kind = match self.kind {
            SemanticSearchKindDto::Document => 0,
            SemanticSearchKindDto::Task => 1,
        };
        let score = self.similarity.to_be_bytes();
        let id = self.id.as_bytes();
        let mut buf = [0_u8; SEMANTIC_CURSOR_BYTES];
        buf[0..4].copy_from_slice(&score);
        buf[4] = kind;
        buf[5..21].copy_from_slice(id);
        URL_SAFE_NO_PAD.encode(buf)
    }

    pub fn decode(raw: &str) -> Option<Self> {
        let bytes = URL_SAFE_NO_PAD.decode(raw).ok()?;
        if bytes.len() != SEMANTIC_CURSOR_BYTES {
            return None;
        }
        let similarity = f32::from_be_bytes(bytes.get(0..4)?.try_into().ok()?);
        let kind = match *bytes.get(4)? {
            0 => SemanticSearchKindDto::Document,
            1 => SemanticSearchKindDto::Task,
            _ => return None,
        };
        let id = Uuid::from_bytes(bytes.get(5..21)?.try_into().ok()?);
        Some(Self {
            similarity,
            kind,
            id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document_hit() -> SemanticSearchHitDto {
        SemanticSearchHitDto {
            id: Uuid::now_v7(),
            kind: SemanticSearchKindDto::Document,
            readable_id: None,
            title: "Incident response".to_owned(),
            project_slug: Some("ops".to_owned()),
            column_name: None,
            similarity: 0.88,
            source: SemanticSearchSourceDto::Content,
            excerpt: "recovery runbook excerpt".to_owned(),
        }
    }

    fn task_hit() -> SemanticSearchHitDto {
        SemanticSearchHitDto {
            id: Uuid::now_v7(),
            kind: SemanticSearchKindDto::Task,
            readable_id: Some("ATL-42".to_owned()),
            title: "Review outage".to_owned(),
            project_slug: Some("ops".to_owned()),
            column_name: Some("In Progress".to_owned()),
            similarity: 0.77,
            source: SemanticSearchSourceDto::Comment,
            excerpt: "customer-impact discussion".to_owned(),
        }
    }

    #[test]
    fn semantic_hit_serializes_compact_document_shape() {
        let json = serde_json::to_value(document_hit()).unwrap();
        assert_eq!(json["kind"], "document");
        assert_eq!(json["source"], "content");
        assert_eq!(json["excerpt"], "recovery runbook excerpt");
        assert!(json.get("readable_id").is_none());
        assert!(json.get("column_name").is_none());
        assert!(json.get("body").is_none());
        assert!(json.get("content").is_none());
        assert!(json.get("updated_at").is_none());
    }

    #[test]
    fn semantic_hit_serializes_task_discovery_fields() {
        let json = serde_json::to_value(task_hit()).unwrap();
        assert_eq!(json["kind"], "task");
        assert_eq!(json["readable_id"], "ATL-42");
        assert_eq!(json["column_name"], "In Progress");
        assert_eq!(json["source"], "comment");
    }

    #[test]
    fn semantic_cursor_round_trips_similarity_kind_and_id() {
        let id = Uuid::now_v7();
        let cursor = SemanticSearchCursor {
            similarity: 0.625,
            kind: SemanticSearchKindDto::Task,
            id,
        };
        let encoded = cursor.encode();
        let decoded = SemanticSearchCursor::decode(&encoded).expect("valid cursor");
        assert_eq!(decoded.kind, SemanticSearchKindDto::Task);
        assert_eq!(decoded.id, id);
        assert!((decoded.similarity - 0.625).abs() < f32::EPSILON);
    }
}
