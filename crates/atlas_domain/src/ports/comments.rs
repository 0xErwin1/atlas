use crate::{
    DomainError, WorkspaceCtx,
    entities::comments::{
        Comment, CommentFeedCursor, CommentFeedEntry, CommentLink, CommentLinkTarget, CommentOwner,
        NewComment,
    },
    ids::CommentId,
    wikilink::CommentLinkCandidate,
};
use async_trait::async_trait;

#[async_trait]
pub trait CommentRepo: Send + Sync {
    async fn create(&self, ctx: &WorkspaceCtx, new: NewComment) -> Result<Comment, DomainError>;

    /// Fetches a single comment scoped to `owner`.
    ///
    /// Returns `DomainError::NotFound` when the comment does not exist, is
    /// soft-deleted, or belongs to a different owner — this is the IDOR guard
    /// preventing cross-task/cross-document comment id lookups.
    async fn get_for_owner(
        &self,
        ctx: &WorkspaceCtx,
        owner: CommentOwner,
        id: CommentId,
    ) -> Result<Comment, DomainError>;

    /// Lists comments for `owner`, oldest-first, with cursor-based pagination.
    ///
    /// `after_id` is the exclusive lower bound (id of the last seen entry).
    /// This ordering is deliberately the inverse of `TaskActivityRepo::list_for_task`
    /// (newest-first): comments read as a conversation, oldest first.
    async fn list_for_owner(
        &self,
        ctx: &WorkspaceCtx,
        owner: CommentOwner,
        after_id: Option<CommentId>,
        limit: u64,
    ) -> Result<Vec<Comment>, DomainError>;

    async fn soft_delete(
        &self,
        ctx: &WorkspaceCtx,
        owner: CommentOwner,
        id: CommentId,
    ) -> Result<(), DomainError>;
}

#[async_trait]
pub trait CommentLinkRepo: Send + Sync {
    async fn replace_for_comment(
        &self,
        ctx: &WorkspaceCtx,
        comment_id: CommentId,
        targets: Vec<CommentLinkTarget>,
    ) -> Result<(), DomainError>;

    async fn remove_for_comment(
        &self,
        ctx: &WorkspaceCtx,
        comment_id: CommentId,
    ) -> Result<(), DomainError>;

    async fn backlinks_for_target(
        &self,
        ctx: &WorkspaceCtx,
        target: CommentLinkTarget,
    ) -> Result<Vec<CommentLink>, DomainError>;

    async fn links_for_comments(
        &self,
        ctx: &WorkspaceCtx,
        comment_ids: &[CommentId],
    ) -> Result<Vec<CommentLink>, DomainError>;

    async fn feed_for_owner(
        &self,
        ctx: &WorkspaceCtx,
        owner: CommentOwner,
        after: Option<CommentFeedCursor>,
        limit: u64,
    ) -> Result<Vec<CommentFeedEntry>, DomainError>;
}

#[async_trait]
pub trait CommentLinkTargetRepo: Send + Sync {
    /// Resolves syntactically valid candidates under workspace ownership only.
    /// Authorization is deliberately applied later by projection callers.
    async fn classify_candidates(
        &self,
        ctx: &WorkspaceCtx,
        candidates: Vec<CommentLinkCandidate>,
    ) -> Result<Vec<CommentLinkTarget>, DomainError>;
}

#[cfg(test)]
mod tests {
    /// Doc-test: the trait is object-safe and its method signatures compile.
    ///
    /// This is intentionally a compile-only test — it has no runtime I/O.
    #[test]
    fn comment_repo_is_object_safe() {
        use super::CommentRepo;
        let _: Option<Box<dyn CommentRepo>> = None;
    }
}
