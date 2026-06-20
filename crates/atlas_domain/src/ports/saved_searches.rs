use crate::{
    DomainError, SavedSearchId, WorkspaceCtx,
    entities::saved_searches::{NewSavedSearch, SavedSearch},
};
use async_trait::async_trait;

#[async_trait]
pub trait SavedSearchRepo: Send + Sync {
    async fn create(
        &self,
        ctx: &WorkspaceCtx,
        new: NewSavedSearch,
    ) -> Result<SavedSearch, DomainError>;

    async fn find(
        &self,
        ctx: &WorkspaceCtx,
        id: SavedSearchId,
    ) -> Result<Option<SavedSearch>, DomainError>;

    async fn list_for_owner(&self, ctx: &WorkspaceCtx) -> Result<Vec<SavedSearch>, DomainError>;

    async fn rename(
        &self,
        ctx: &WorkspaceCtx,
        id: SavedSearchId,
        new_name: String,
    ) -> Result<SavedSearch, DomainError>;

    async fn delete(&self, ctx: &WorkspaceCtx, id: SavedSearchId) -> Result<(), DomainError>;
}
