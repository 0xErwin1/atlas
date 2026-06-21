use crate::{
    DomainError, TaskViewId, WorkspaceCtx,
    entities::task_views::{NewTaskView, TaskView, TaskViewFilters},
};
use async_trait::async_trait;

#[async_trait]
pub trait TaskViewRepo: Send + Sync {
    async fn create(&self, ctx: &WorkspaceCtx, new: NewTaskView) -> Result<TaskView, DomainError>;

    async fn find(
        &self,
        ctx: &WorkspaceCtx,
        id: TaskViewId,
    ) -> Result<Option<TaskView>, DomainError>;

    async fn list_for_owner(&self, ctx: &WorkspaceCtx) -> Result<Vec<TaskView>, DomainError>;

    async fn update(
        &self,
        ctx: &WorkspaceCtx,
        id: TaskViewId,
        name: String,
        filters: TaskViewFilters,
    ) -> Result<TaskView, DomainError>;

    async fn delete(&self, ctx: &WorkspaceCtx, id: TaskViewId) -> Result<(), DomainError>;
}
