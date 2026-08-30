use crate::{
    DomainError, WorkspaceCtx,
    entities::boards_tasks::PositionBetween,
    entities::status_templates::{
        NewStatusTemplate, PlatformStatusTemplate, StatusTemplate, StatusTemplatePatch,
    },
    ids::{PlatformStatusTemplateId, StatusTemplateId},
};
use async_trait::async_trait;

#[async_trait]
pub trait StatusTemplateRepo: Send + Sync {
    async fn create(
        &self,
        ctx: &WorkspaceCtx,
        new: NewStatusTemplate,
    ) -> Result<StatusTemplate, DomainError>;

    async fn list(&self, ctx: &WorkspaceCtx) -> Result<Vec<StatusTemplate>, DomainError>;

    async fn patch(
        &self,
        ctx: &WorkspaceCtx,
        id: StatusTemplateId,
        patch: StatusTemplatePatch,
    ) -> Result<StatusTemplate, DomainError>;

    async fn move_template(
        &self,
        ctx: &WorkspaceCtx,
        id: StatusTemplateId,
        position: PositionBetween,
    ) -> Result<(), DomainError>;

    async fn soft_delete(
        &self,
        ctx: &WorkspaceCtx,
        id: StatusTemplateId,
    ) -> Result<(), DomainError>;
}

/// Platform-level default statuses. Deliberately without `WorkspaceCtx`: these
/// rows belong to the Atlas instance, not to a tenant, so the multi-tenant
/// scoping the workspace port enforces does not apply. Callers are gated by the
/// admin-only HTTP surface instead.
#[async_trait]
pub trait PlatformStatusTemplateRepo: Send + Sync {
    async fn create(&self, new: NewStatusTemplate) -> Result<PlatformStatusTemplate, DomainError>;

    async fn list(&self) -> Result<Vec<PlatformStatusTemplate>, DomainError>;

    async fn patch(
        &self,
        id: PlatformStatusTemplateId,
        patch: StatusTemplatePatch,
    ) -> Result<PlatformStatusTemplate, DomainError>;

    async fn move_template(
        &self,
        id: PlatformStatusTemplateId,
        position: PositionBetween,
    ) -> Result<(), DomainError>;

    async fn soft_delete(&self, id: PlatformStatusTemplateId) -> Result<(), DomainError>;
}
