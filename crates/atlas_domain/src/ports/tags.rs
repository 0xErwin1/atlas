use crate::{
    DomainError, WorkspaceCtx,
    entities::tags::{NewTag, Tag},
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
}
