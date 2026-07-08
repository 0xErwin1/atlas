use async_trait::async_trait;
use uuid::Uuid;

use crate::{DomainError, WorkspaceCtx, permissions::Principal};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResourceKind {
    Document,
    Task,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SemanticSearchSource {
    Title,
    Content,
    Comment,
    AttachmentName,
    Checklist,
    Subtask,
    Aggregate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SemanticSearchTypeFilter {
    pub documents: bool,
    pub tasks: bool,
}

impl Default for SemanticSearchTypeFilter {
    fn default() -> Self {
        Self::all()
    }
}

impl SemanticSearchTypeFilter {
    pub const fn all() -> Self {
        Self {
            documents: true,
            tasks: true,
        }
    }

    pub const fn documents() -> Self {
        Self {
            documents: true,
            tasks: false,
        }
    }

    pub const fn tasks() -> Self {
        Self {
            documents: false,
            tasks: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct SemanticCursorSortTuple {
    pub similarity: f32,
    pub resource_kind: ResourceKind,
    pub resource_id: Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SemanticSearchAfter {
    pub similarity: f32,
    pub resource_kind: ResourceKind,
    pub resource_id: Uuid,
}

impl SemanticSearchAfter {
    pub const fn new(similarity: f32, resource_kind: ResourceKind, resource_id: Uuid) -> Self {
        Self {
            similarity,
            resource_kind,
            resource_id,
        }
    }

    pub const fn sort_tuple(self) -> SemanticCursorSortTuple {
        SemanticCursorSortTuple {
            similarity: self.similarity,
            resource_kind: self.resource_kind,
            resource_id: self.resource_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticSearchQuery {
    pub workspace_id: crate::WorkspaceId,
    pub principal: Principal,
    pub text: String,
    pub type_filter: SemanticSearchTypeFilter,
    pub limit: u64,
    pub after: Option<SemanticSearchAfter>,
    pub bypass: bool,
    pub may_read_documents: bool,
    pub may_read_tasks: bool,
}

impl SemanticSearchQuery {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        workspace_id: crate::WorkspaceId,
        principal: Principal,
        text: String,
        type_filter: SemanticSearchTypeFilter,
        limit: u64,
        after: Option<SemanticSearchAfter>,
        bypass: bool,
        may_read_documents: bool,
        may_read_tasks: bool,
    ) -> Self {
        Self {
            workspace_id,
            principal,
            text,
            type_filter,
            limit,
            after,
            bypass,
            may_read_documents,
            may_read_tasks,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticSearchHit {
    pub kind: ResourceKind,
    pub id: Uuid,
    pub readable_id: Option<String>,
    pub title: String,
    pub project_slug: Option<String>,
    pub column_name: Option<String>,
    pub similarity: f32,
    pub source: SemanticSearchSource,
    pub excerpt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingInput {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SemanticIndexChunk {
    pub workspace_id: crate::WorkspaceId,
    pub kind: ResourceKind,
    pub resource_id: Uuid,
    pub source: SemanticSearchSource,
    pub chunk_ordinal: i32,
    pub content_hash: String,
    pub text: String,
    pub excerpt: String,
}

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, inputs: &[EmbeddingInput]) -> Result<Vec<Vec<f32>>, DomainError>;

    fn model(&self) -> &str;

    fn dimensions(&self) -> usize;
}

#[async_trait]
pub trait SemanticIndexer: Send + Sync {
    async fn index_resource(
        &self,
        ctx: &WorkspaceCtx,
        kind: ResourceKind,
        resource_id: Uuid,
    ) -> Result<(), DomainError>;
}

#[async_trait]
pub trait SemanticSearchRepo: Send + Sync {
    async fn search(
        &self,
        query: &SemanticSearchQuery,
    ) -> Result<Vec<SemanticSearchHit>, DomainError>;
}
