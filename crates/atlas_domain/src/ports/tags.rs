use crate::{
    DomainError, WorkspaceCtx,
    entities::tags::{NewTag, Tag},
    ids::TagId,
};
use async_trait::async_trait;

#[async_trait]
pub trait TagRepo: Send + Sync {
    async fn create(&self, ctx: &WorkspaceCtx, new: NewTag) -> Result<Tag, DomainError>;

    async fn find_by_name(
        &self,
        ctx: &WorkspaceCtx,
        name: &str,
    ) -> Result<Option<Tag>, DomainError>;

    async fn list(&self, ctx: &WorkspaceCtx) -> Result<Vec<Tag>, DomainError>;

    /// Updates a tag's name and/or color in a single transaction.
    ///
    /// When `name` changes, all task labels in the workspace are backfilled from
    /// the old name to the new name atomically, including a dedup pass for tasks
    /// that already carry both values.
    ///
    /// Returns `DomainError::NotFound` if `id` does not belong to this workspace
    /// or has been soft-deleted. Returns `DomainError::Conflict` on a duplicate
    /// lowercase name.
    async fn update(
        &self,
        ctx: &WorkspaceCtx,
        id: TagId,
        name: Option<String>,
        color: Option<String>,
    ) -> Result<Tag, DomainError>;

    /// Soft-deletes a tag by setting `deleted_at = now()`.
    ///
    /// Task label strings are NOT affected — labels are free strings, not foreign
    /// keys. Returns `DomainError::NotFound` if `id` does not belong to this
    /// workspace or has already been deleted.
    async fn soft_delete(&self, ctx: &WorkspaceCtx, id: TagId) -> Result<(), DomainError>;
}
