use crate::{
    DomainError, WorkspaceCtx,
    entities::boards_tasks::PositionBetween,
    entities::status_templates::{NewStatusTemplate, StatusTemplate, StatusTemplatePatch},
    ids::StatusTemplateId,
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
