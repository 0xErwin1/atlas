use crate::ids::{
    ApiKeyId, AttachmentId, CommentDraftId, CommentId, DocumentId, FolderId, ProjectId, RevisionId,
    UserId, WorkspaceId,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: DocumentId,
    pub workspace_id: WorkspaceId,
    pub project_id: Option<ProjectId>,
    pub folder_id: Option<FolderId>,
    pub title: String,
    pub slug: Option<String>,
    pub content: String,
    pub frontmatter: serde_json::Value,
    pub current_revision_id: RevisionId,
    pub current_revision_seq: i64,
    pub created_by_user_id: Option<UserId>,
    pub created_by_api_key_id: Option<ApiKeyId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSummary {
    pub id: DocumentId,
    pub workspace_id: WorkspaceId,
    pub project_id: Option<ProjectId>,
    pub folder_id: Option<FolderId>,
    pub title: String,
    pub slug: Option<String>,
    pub frontmatter: serde_json::Value,
    pub current_revision_id: RevisionId,
    pub current_revision_seq: i64,
    pub created_by_user_id: Option<UserId>,
    pub created_by_api_key_id: Option<ApiKeyId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct DocumentFilter {
    pub project_id: Option<ProjectId>,
    pub folder_id: Option<FolderId>,
}

#[derive(Debug, Clone)]
pub struct NewDocument {
    pub title: String,
    pub slug: Option<String>,
    pub content: String,
    pub folder_id: Option<FolderId>,
    pub project_id: Option<ProjectId>,
    pub frontmatter: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentRevision {
    pub id: RevisionId,
    pub workspace_id: WorkspaceId,
    pub document_id: DocumentId,
    pub seq: i64,
    pub patch: Option<String>,
    pub snapshot: Option<String>,
    pub is_anchor: bool,
    pub created_by_user_id: Option<UserId>,
    pub created_by_api_key_id: Option<ApiKeyId>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevisionMeta {
    pub id: RevisionId,
    pub seq: i64,
    pub is_anchor: bool,
    pub created_by_user_id: Option<UserId>,
    pub created_by_api_key_id: Option<ApiKeyId>,
    pub created_at: DateTime<Utc>,
}

/// The owning source of a document link (polymorphic: doc or task).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkSource {
    Document(DocumentId),
    Task(crate::ids::TaskId),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentLink {
    pub id: crate::ids::DocumentId,
    pub workspace_id: WorkspaceId,
    pub source_document_id: Option<DocumentId>,
    pub source_task_id: Option<crate::ids::TaskId>,
    pub target_document_id: Option<DocumentId>,
    pub target_title: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct ExtractedLink {
    pub target_title: String,
    pub target_document_id: Option<DocumentId>,
}

#[derive(Debug, Clone)]
pub struct TaskDescriptionLinks {
    pub description: String,
    pub links: Vec<DocumentLink>,
}

#[derive(Debug, Clone)]
pub struct RankedTaskDescriptionLink {
    pub link: DocumentLink,
    pub rank: usize,
}

/// Returns one deterministic representative per resolved document or unresolved title.
pub fn rank_task_description_links(
    snapshot: &TaskDescriptionLinks,
) -> Vec<RankedTaskDescriptionLink> {
    let ranks = description_link_ranks(&snapshot.description);
    let mut groups: Vec<(LinkIdentity, RankedTaskDescriptionLink)> = Vec::new();

    for link in &snapshot.links {
        let normalized_title = normalize_link_title(&link.target_title);
        let Some(rank) = ranks.get(&normalized_title).copied() else {
            continue;
        };

        let identity = match link.target_document_id {
            Some(id) => LinkIdentity::Resolved(id),
            None => LinkIdentity::Unresolved(normalized_title),
        };

        let candidate = RankedTaskDescriptionLink {
            link: link.clone(),
            rank,
        };

        if let Some((_, existing)) = groups.iter_mut().find(|(key, _)| *key == identity) {
            existing.rank = existing.rank.min(candidate.rank);
            if link_representative_key(&candidate.link) < link_representative_key(&existing.link) {
                existing.link = candidate.link;
            }
        } else {
            groups.push((identity, candidate));
        }
    }

    let mut ranked: Vec<_> = groups.into_iter().map(|(_, link)| link).collect();
    ranked.sort_by_key(|entry| (entry.rank, entry.link.id.0));
    ranked
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LinkIdentity {
    Resolved(DocumentId),
    Unresolved(String),
}

fn description_link_ranks(description: &str) -> std::collections::HashMap<String, usize> {
    crate::wikilink::parse_wikilinks(description)
        .into_iter()
        .enumerate()
        .fold(
            std::collections::HashMap::new(),
            |mut ranks, (index, raw)| {
                let (_, title) = crate::wikilink::parse_wikilink_target(&raw);
                ranks
                    .entry(normalize_link_title(&title))
                    .and_modify(|rank| *rank = (*rank).min(index))
                    .or_insert(index);
                ranks
            },
        )
}

fn normalize_link_title(title: &str) -> String {
    title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn link_representative_key(link: &DocumentLink) -> (DateTime<Utc>, uuid::Uuid) {
    (link.created_at, link.id.0)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub id: AttachmentId,
    pub workspace_id: WorkspaceId,
    pub document_id: Option<DocumentId>,
    pub task_id: Option<crate::ids::TaskId>,
    pub comment_id: Option<CommentId>,
    pub draft_id: Option<CommentDraftId>,
    pub file_name: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub created_by_user_id: Option<UserId>,
    pub created_by_api_key_id: Option<ApiKeyId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewAttachment {
    pub document_id: Option<DocumentId>,
    pub task_id: Option<crate::ids::TaskId>,
    pub comment_id: Option<CommentId>,
    pub file_name: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub enum AttachmentOwner {
    Document(DocumentId),
    Task(crate::ids::TaskId),
    Comment(CommentId),
    Draft(CommentDraftId),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentWriteIntent {
    pub id: uuid::Uuid,
    pub digest: String,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_task_description_links_uses_earliest_alias_and_representative() {
        let resolved = DocumentId::new();
        let earliest = DocumentId::new();
        let later = DocumentId::new();
        let workspace = WorkspaceId::new();
        let task = crate::ids::TaskId::new();
        let before = Utc::now() - chrono::TimeDelta::seconds(1);
        let now = Utc::now();

        let ranked = rank_task_description_links(&TaskDescriptionLinks {
            description: "[[Second Alias]] then [[First Alias]]".into(),
            links: vec![
                DocumentLink {
                    id: later,
                    workspace_id: workspace,
                    source_document_id: None,
                    source_task_id: Some(task),
                    target_document_id: Some(resolved),
                    target_title: "First Alias".into(),
                    created_at: now,
                },
                DocumentLink {
                    id: earliest,
                    workspace_id: workspace,
                    source_document_id: None,
                    source_task_id: Some(task),
                    target_document_id: Some(resolved),
                    target_title: "Second Alias".into(),
                    created_at: before,
                },
            ],
        });

        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].rank, 0);
        assert_eq!(ranked[0].link.id, earliest);
        assert_eq!(ranked[0].link.target_document_id, Some(resolved));
    }

    #[test]
    fn rank_task_description_links_normalizes_unresolved_titles_without_alias_matching() {
        let workspace = WorkspaceId::new();
        let task = crate::ids::TaskId::new();
        let first = DocumentId::new();
        let second = DocumentId::new();
        let created_at = Utc::now();

        let ranked = rank_task_description_links(&TaskDescriptionLinks {
            description: "[[Missing Doc]] [[ missing   doc ]] [[Other Missing]]".into(),
            links: vec![
                DocumentLink {
                    id: first,
                    workspace_id: workspace,
                    source_document_id: None,
                    source_task_id: Some(task),
                    target_document_id: None,
                    target_title: "Missing Doc".into(),
                    created_at,
                },
                DocumentLink {
                    id: second,
                    workspace_id: workspace,
                    source_document_id: None,
                    source_task_id: Some(task),
                    target_document_id: None,
                    target_title: "Other Missing".into(),
                    created_at,
                },
            ],
        });

        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].link.id, first);
        assert_eq!(ranked[0].rank, 0);
        assert_eq!(ranked[1].link.id, second);
        assert_eq!(ranked[1].rank, 2);
    }
}
