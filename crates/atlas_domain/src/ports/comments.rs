use crate::{
    DomainError, WorkspaceCtx,
    entities::comments::{Comment, CommentOwner, NewComment},
    ids::CommentId,
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
