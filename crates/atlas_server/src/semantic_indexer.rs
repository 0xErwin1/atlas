use atlas_domain::{
    ids::WorkspaceId,
    semantic_search::{ResourceKind, SemanticIndexChunk, SemanticSearchSource},
};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentText {
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentText {
    pub file_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecklistText {
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubtaskText {
    pub readable_id: String,
    pub title: String,
    pub description: String,
    pub checklist_items: Vec<ChecklistText>,
}

#[derive(Debug, Clone)]
pub struct TaskIndexInput {
    pub workspace_id: WorkspaceId,
    pub task_id: uuid::Uuid,
    pub readable_id: String,
    pub title: String,
    pub description: String,
    pub labels: Vec<String>,
    pub comments: Vec<CommentText>,
    pub attachments: Vec<AttachmentText>,
    pub checklist_items: Vec<ChecklistText>,
    pub subtasks: Vec<SubtaskText>,
    pub max_chunk_chars: usize,
}

#[derive(Debug, Clone)]
pub struct DocumentIndexInput {
    pub workspace_id: WorkspaceId,
    pub document_id: uuid::Uuid,
    pub title: String,
    pub content: String,
    pub comments: Vec<CommentText>,
    pub attachments: Vec<AttachmentText>,
    pub max_chunk_chars: usize,
}

pub fn aggregate_task_chunks(input: TaskIndexInput) -> Vec<SemanticIndexChunk> {
    let text = join_non_empty(task_parts(&input));
    chunk_semantic_text(
        input.workspace_id,
        ResourceKind::Task,
        input.task_id,
        SemanticSearchSource::Aggregate,
        &text,
        input.max_chunk_chars,
    )
}

pub fn aggregate_document_chunks(input: DocumentIndexInput) -> Vec<SemanticIndexChunk> {
    let mut parts = vec![input.title, input.content];
    parts.extend(input.comments.into_iter().map(|comment| comment.body));
    parts.extend(
        input
            .attachments
            .into_iter()
            .map(|attachment| attachment.file_name),
    );
    let text = join_non_empty(parts);
    chunk_semantic_text(
        input.workspace_id,
        ResourceKind::Document,
        input.document_id,
        SemanticSearchSource::Content,
        &text,
        input.max_chunk_chars,
    )
}

pub fn chunk_semantic_text(
    workspace_id: WorkspaceId,
    kind: ResourceKind,
    resource_id: uuid::Uuid,
    source: SemanticSearchSource,
    text: &str,
    max_chunk_chars: usize,
) -> Vec<SemanticIndexChunk> {
    let normalized = normalize_text(text);
    if normalized.is_empty() {
        return Vec::new();
    }

    let max_chars = max_chunk_chars.max(1);
    split_on_word_boundaries(&normalized, max_chars)
        .into_iter()
        .enumerate()
        .map(|(idx, chunk_text)| SemanticIndexChunk {
            workspace_id,
            kind,
            resource_id,
            source,
            chunk_ordinal: idx as i32,
            content_hash: content_hash(&chunk_text),
            excerpt: excerpt(&chunk_text),
            text: chunk_text,
        })
        .collect()
}

pub fn document_content_hash(title: &str, content: &str) -> String {
    content_hash(&join_non_empty([title.to_owned(), content.to_owned()]))
}

pub fn should_skip_embedding(new_hash: &str, existing_hash: Option<&str>) -> bool {
    existing_hash.is_some_and(|existing| existing == new_hash)
}

fn task_parts(input: &TaskIndexInput) -> Vec<String> {
    let mut parts = vec![
        input.readable_id.clone(),
        input.title.clone(),
        input.description.clone(),
    ];
    parts.extend(input.labels.iter().cloned());
    parts.extend(input.comments.iter().map(|comment| comment.body.clone()));
    parts.extend(
        input
            .attachments
            .iter()
            .map(|attachment| attachment.file_name.clone()),
    );
    parts.extend(input.checklist_items.iter().map(|item| item.title.clone()));
    for subtask in &input.subtasks {
        parts.push(subtask.readable_id.clone());
        parts.push(subtask.title.clone());
        parts.push(subtask.description.clone());
        parts.extend(
            subtask
                .checklist_items
                .iter()
                .map(|item| item.title.clone()),
        );
    }
    parts
}

fn normalize_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn split_on_word_boundaries(text: &str, max_chars: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();

    for word in text.split_whitespace() {
        if current.is_empty() {
            if word.chars().count() <= max_chars {
                current.push_str(word);
            } else {
                chunks.extend(split_long_word(word, max_chars));
            }
            continue;
        }

        let candidate_len = current.chars().count() + 1 + word.chars().count();
        if candidate_len <= max_chars {
            current.push(' ');
            current.push_str(word);
        } else {
            chunks.push(std::mem::take(&mut current));
            if word.chars().count() <= max_chars {
                current.push_str(word);
            } else {
                chunks.extend(split_long_word(word, max_chars));
            }
        }
    }

    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn split_long_word(word: &str, max_chars: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    for ch in word.chars() {
        if current.chars().count() == max_chars {
            chunks.push(std::mem::take(&mut current));
        }
        current.push(ch);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn join_non_empty(parts: impl IntoIterator<Item = String>) -> String {
    parts
        .into_iter()
        .map(|part| normalize_text(&part))
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn content_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(normalize_text(text).as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn excerpt(text: &str) -> String {
    const MAX_EXCERPT_CHARS: usize = 240;
    text.chars().take(MAX_EXCERPT_CHARS).collect()
}
