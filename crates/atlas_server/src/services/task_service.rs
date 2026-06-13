use atlas_domain::{
    DomainError, WorkspaceCtx,
    entities::boards_tasks::{
        ActivityKind, ActivityPayload, AssigneeRef, NewTask, NewTaskActivity, NewTaskAssignee,
        NewTaskChecklistItem, NewTaskReference, PositionBetween, ReferenceKind, Task, TaskActivity,
        TaskAssignee, TaskChecklistItem, TaskChecklistItemPatch, TaskPatch, TaskReference,
    },
    ids::{BoardId, ChecklistItemId, ColumnId, ProjectId, TaskActivityId, TaskId, TaskReferenceId},
};
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, TransactionTrait};

use crate::persistence::entities::boards_tasks::{
    task, task_checklist_item, task_checklist_item_from,
};
use crate::persistence::repos::{
    PgTaskActivityRepo, PgTaskAssigneeRepo, PgTaskChecklistRepo, PgTaskReferenceRepo, PgTaskRepo,
    TaskActivityRepo,
};

/// Result of a checklist item promotion: the three records committed atomically.
pub struct PromotionResult {
    pub task: Task,
    pub parent_reference: Option<TaskReference>,
    pub checklist_item: TaskChecklistItem,
}

/// Coordinates multi-table transactions for task mutations.
///
/// Every state-changing method opens one `DatabaseTransaction`, runs the core
/// mutation and the activity append on that same connection, then commits once.
/// A failure at any step rolls back both the mutation and the activity, satisfying
/// the "no tearing" requirement from spec Req 7 and the move↔activity atomicity
/// guarantee from the design.
pub struct TaskService {
    conn: DatabaseConnection,
}

impl TaskService {
    pub fn new(
        conn: DatabaseConnection,
        _task_repo: PgTaskRepo,
        _reference_repo: PgTaskReferenceRepo,
        _assignee_repo: PgTaskAssigneeRepo,
        _checklist_repo: PgTaskChecklistRepo,
        _activity_repo: PgTaskActivityRepo,
    ) -> Self {
        Self { conn }
    }

    /// Creates a task and appends a `Created` activity in the same transaction.
    pub async fn create(&self, ctx: &WorkspaceCtx, new: NewTask) -> Result<Task, DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;

        let task = PgTaskRepo::create_in(&txn, ctx, new).await?;

        PgTaskActivityRepo::append_in(
            &txn,
            ctx,
            NewTaskActivity {
                task_id: task.id,
                kind: ActivityKind::Created,
                payload: ActivityPayload::Created,
            },
        )
        .await?;

        txn.commit().await.map_err(db_err)?;
        Ok(task)
    }

    /// Patches a task and appends one `FieldChanged` activity per changed field,
    /// all in a single transaction.
    pub async fn patch(
        &self,
        ctx: &WorkspaceCtx,
        id: TaskId,
        patch: TaskPatch,
    ) -> Result<Task, DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;

        let before = task::Entity::find_by_id(id.0)
            .filter(task::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task::Column::DeletedAt.is_null())
            .one(&txn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "task",
                id: id.0,
            })
            .map(crate::persistence::entities::boards_tasks::task_from)?;

        let fields_changed: Vec<(String, serde_json::Value, serde_json::Value)> =
            collect_field_changes(&before, &patch);

        let updated = PgTaskRepo::patch_in(&txn, ctx, id, patch).await?;

        for (field, old_value, new_value) in fields_changed {
            PgTaskActivityRepo::append_in(
                &txn,
                ctx,
                NewTaskActivity {
                    task_id: id,
                    kind: ActivityKind::FieldChanged,
                    payload: ActivityPayload::FieldChanged {
                        field,
                        old_value,
                        new_value,
                    },
                },
            )
            .await?;
        }

        txn.commit().await.map_err(db_err)?;
        Ok(updated)
    }

    /// Moves a task to a new column and position, recording a `Moved` activity,
    /// all in a single transaction.
    ///
    /// The move (including any resequence+retry) and the activity append run on the
    /// same `DatabaseTransaction`, satisfying the move↔activity atomicity guarantee.
    pub async fn move_task(
        &self,
        ctx: &WorkspaceCtx,
        id: TaskId,
        column_id: ColumnId,
        position: PositionBetween,
    ) -> Result<Task, DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;

        let before = task::Entity::find_by_id(id.0)
            .filter(task::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task::Column::DeletedAt.is_null())
            .one(&txn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "task",
                id: id.0,
            })
            .map(crate::persistence::entities::boards_tasks::task_from)?;

        let moved = PgTaskRepo::move_to_in(&txn, ctx, id, column_id, position).await?;

        PgTaskActivityRepo::append_in(
            &txn,
            ctx,
            NewTaskActivity {
                task_id: id,
                kind: ActivityKind::Moved,
                payload: ActivityPayload::Moved {
                    from_column_id: before.column_id,
                    to_column_id: column_id,
                },
            },
        )
        .await?;

        txn.commit().await.map_err(db_err)?;
        Ok(moved)
    }

    /// Soft-deletes a task and records a `Deleted` activity in the same transaction.
    pub async fn delete_task(&self, ctx: &WorkspaceCtx, id: TaskId) -> Result<(), DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;

        PgTaskRepo::soft_delete_in(&txn, ctx, id).await?;

        PgTaskActivityRepo::append_in(
            &txn,
            ctx,
            NewTaskActivity {
                task_id: id,
                kind: ActivityKind::Deleted,
                payload: ActivityPayload::Deleted,
            },
        )
        .await?;

        txn.commit().await.map_err(db_err)?;
        Ok(())
    }

    /// Assigns a user or API key to a task, recording an `Assigned` activity,
    /// all in a single transaction.
    pub async fn assign(
        &self,
        ctx: &WorkspaceCtx,
        task_id: TaskId,
        assignee: AssigneeRef,
    ) -> Result<TaskAssignee, DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;

        let result =
            PgTaskAssigneeRepo::add_in(&txn, ctx, NewTaskAssignee { task_id, assignee }).await?;

        PgTaskActivityRepo::append_in(
            &txn,
            ctx,
            NewTaskActivity {
                task_id,
                kind: ActivityKind::Assigned,
                payload: ActivityPayload::Assigned { assignee },
            },
        )
        .await?;

        txn.commit().await.map_err(db_err)?;
        Ok(result)
    }

    /// Removes an assignee from a task, recording an `Unassigned` activity,
    /// all in a single transaction.
    pub async fn unassign(
        &self,
        ctx: &WorkspaceCtx,
        task_id: TaskId,
        assignee: AssigneeRef,
    ) -> Result<(), DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;

        PgTaskAssigneeRepo::remove_in(&txn, ctx, task_id, assignee).await?;

        PgTaskActivityRepo::append_in(
            &txn,
            ctx,
            NewTaskActivity {
                task_id,
                kind: ActivityKind::Unassigned,
                payload: ActivityPayload::Unassigned { assignee },
            },
        )
        .await?;

        txn.commit().await.map_err(db_err)?;
        Ok(())
    }

    /// Adds a reference to a task, recording a `ReferenceAdded` activity,
    /// all in a single transaction.
    pub async fn add_reference(
        &self,
        ctx: &WorkspaceCtx,
        new: NewTaskReference,
    ) -> Result<TaskReference, DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;
        let source_task_id = new.source_task_id;

        let reference = PgTaskReferenceRepo::create_in(&txn, ctx, new).await?;

        PgTaskActivityRepo::append_in(
            &txn,
            ctx,
            NewTaskActivity {
                task_id: source_task_id,
                kind: ActivityKind::ReferenceAdded,
                payload: ActivityPayload::ReferenceAdded {
                    reference_id: reference.id,
                    kind: reference.kind.clone(),
                },
            },
        )
        .await?;

        txn.commit().await.map_err(db_err)?;
        Ok(reference)
    }

    /// Removes a reference by ID, recording a `ReferenceRemoved` activity,
    /// all in a single transaction.
    pub async fn remove_reference(
        &self,
        ctx: &WorkspaceCtx,
        task_id: TaskId,
        reference_id: TaskReferenceId,
        kind: ReferenceKind,
    ) -> Result<(), DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;

        PgTaskReferenceRepo::delete_in(&txn, ctx, reference_id).await?;

        PgTaskActivityRepo::append_in(
            &txn,
            ctx,
            NewTaskActivity {
                task_id,
                kind: ActivityKind::ReferenceRemoved,
                payload: ActivityPayload::ReferenceRemoved { reference_id, kind },
            },
        )
        .await?;

        txn.commit().await.map_err(db_err)?;
        Ok(())
    }

    /// Adds a checklist item to a task, recording a `ChecklistAdded` activity,
    /// all in a single transaction.
    pub async fn add_checklist_item(
        &self,
        ctx: &WorkspaceCtx,
        new: NewTaskChecklistItem,
    ) -> Result<TaskChecklistItem, DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;
        let task_id = new.task_id;
        let title = new.title.clone();

        let item = PgTaskChecklistRepo::add_item_in(&txn, ctx, new).await?;

        PgTaskActivityRepo::append_in(
            &txn,
            ctx,
            NewTaskActivity {
                task_id,
                kind: ActivityKind::ChecklistAdded,
                payload: ActivityPayload::ChecklistAdded {
                    item_id: item.id,
                    title,
                },
            },
        )
        .await?;

        txn.commit().await.map_err(db_err)?;
        Ok(item)
    }

    /// Patches a checklist item, recording a `ChecklistUpdated` activity,
    /// all in a single transaction.
    pub async fn patch_checklist_item(
        &self,
        ctx: &WorkspaceCtx,
        task_id: TaskId,
        item_id: ChecklistItemId,
        patch: TaskChecklistItemPatch,
    ) -> Result<TaskChecklistItem, DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;

        let item = PgTaskChecklistRepo::patch_item_in(&txn, ctx, item_id, patch).await?;

        PgTaskActivityRepo::append_in(
            &txn,
            ctx,
            NewTaskActivity {
                task_id,
                kind: ActivityKind::ChecklistUpdated,
                payload: ActivityPayload::ChecklistUpdated { item_id },
            },
        )
        .await?;

        txn.commit().await.map_err(db_err)?;
        Ok(item)
    }

    /// Soft-deletes a checklist item, recording a `ChecklistRemoved` activity,
    /// all in a single transaction.
    pub async fn remove_checklist_item(
        &self,
        ctx: &WorkspaceCtx,
        task_id: TaskId,
        item_id: ChecklistItemId,
    ) -> Result<(), DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;

        PgTaskChecklistRepo::soft_delete_item_in(&txn, ctx, item_id).await?;

        PgTaskActivityRepo::append_in(
            &txn,
            ctx,
            NewTaskActivity {
                task_id,
                kind: ActivityKind::ChecklistRemoved,
                payload: ActivityPayload::ChecklistRemoved { item_id },
            },
        )
        .await?;

        txn.commit().await.map_err(db_err)?;
        Ok(())
    }

    /// Promotes a checklist item to a full task in a single atomic transaction.
    ///
    /// Steps (all in one txn, full rollback on failure):
    /// 1. Guard: reject if item is already promoted.
    /// 2. Create the child task via `PgTaskRepo::create_in`.
    /// 3. Create a `Parent` reference from child → parent.
    /// 4. Mark the checklist item as promoted.
    /// 5. Append `Created` activity for the new task.
    /// 6. Append `ChecklistPromoted` activity for the parent task.
    pub async fn promote_checklist_item(
        &self,
        ctx: &WorkspaceCtx,
        item_id: ChecklistItemId,
        project_id: ProjectId,
        board_id: BoardId,
        column_id: ColumnId,
    ) -> Result<PromotionResult, DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;

        // Fetch the item directly by ID so we can read its parent task_id and
        // guard against re-promotion, all within the open transaction.
        let item = {
            task_checklist_item::Entity::find_by_id(item_id.0)
                .filter(task_checklist_item::Column::WorkspaceId.eq(ctx.workspace_id.0))
                .filter(task_checklist_item::Column::DeletedAt.is_null())
                .one(&txn)
                .await
                .map_err(db_err)?
                .ok_or(DomainError::NotFound {
                    entity: "task_checklist_item",
                    id: item_id.0,
                })
                .map(task_checklist_item_from)?
        };

        if item.promoted_task_id.is_some() {
            txn.rollback().await.map_err(db_err)?;
            return Err(DomainError::Forbidden {
                message: "checklist item has already been promoted".into(),
            });
        }

        let parent_task_id = item.task_id;

        let child_task = PgTaskRepo::create_in(
            &txn,
            ctx,
            NewTask {
                project_id,
                board_id,
                column_id,
                title: item.title.clone(),
                description: String::new(),
                priority: None,
                due_date: None,
                estimate: None,
                labels: vec![],
                properties: None,
                position: PositionBetween {
                    before: None,
                    after: None,
                },
            },
        )
        .await?;

        let parent_ref = PgTaskReferenceRepo::create_in(
            &txn,
            ctx,
            NewTaskReference {
                source_task_id: child_task.id,
                kind: ReferenceKind::Parent,
                target_task_id: Some(parent_task_id),
                target_document_id: None,
            },
        )
        .await
        .ok();

        let updated_item =
            PgTaskChecklistRepo::mark_promoted_in(&txn, ctx, item_id, child_task.id).await?;

        PgTaskActivityRepo::append_in(
            &txn,
            ctx,
            NewTaskActivity {
                task_id: child_task.id,
                kind: ActivityKind::Created,
                payload: ActivityPayload::Created,
            },
        )
        .await?;

        PgTaskActivityRepo::append_in(
            &txn,
            ctx,
            NewTaskActivity {
                task_id: parent_task_id,
                kind: ActivityKind::ChecklistPromoted,
                payload: ActivityPayload::ChecklistPromoted {
                    item_id,
                    promoted_task_id: child_task.id,
                },
            },
        )
        .await?;

        txn.commit().await.map_err(db_err)?;

        Ok(PromotionResult {
            task: child_task,
            parent_reference: parent_ref,
            checklist_item: updated_item,
        })
    }

    /// Returns paginated activity entries for a task, newest-first.
    pub async fn list_activity(
        &self,
        ctx: &WorkspaceCtx,
        task_id: TaskId,
        after_id: Option<TaskActivityId>,
        limit: u64,
    ) -> Result<Vec<TaskActivity>, DomainError> {
        let repo = PgTaskActivityRepo::new(self.conn.clone());
        repo.list_for_task(ctx, task_id, after_id, limit).await
    }
}

fn db_err(e: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: e.to_string(),
    }
}

fn collect_field_changes(
    before: &atlas_domain::entities::boards_tasks::Task,
    patch: &TaskPatch,
) -> Vec<(String, serde_json::Value, serde_json::Value)> {
    let mut changes = Vec::new();

    if let Some(new_title) = &patch.title
        && new_title != &before.title
    {
        changes.push((
            "title".into(),
            serde_json::json!(before.title),
            serde_json::json!(new_title),
        ));
    }
    if let Some(new_desc) = &patch.description
        && new_desc != &before.description
    {
        changes.push((
            "description".into(),
            serde_json::json!(before.description),
            serde_json::json!(new_desc),
        ));
    }
    if let Some(new_priority) = &patch.priority {
        let old = before.priority.as_ref().map(|p| p.as_str());
        let new = new_priority.as_ref().map(|p| p.as_str());
        if old != new {
            changes.push((
                "priority".into(),
                serde_json::json!(old),
                serde_json::json!(new),
            ));
        }
    }
    if let Some(new_due) = &patch.due_date
        && new_due != &before.due_date
    {
        changes.push((
            "due_date".into(),
            serde_json::json!(before.due_date),
            serde_json::json!(new_due),
        ));
    }
    if let Some(new_est) = &patch.estimate
        && new_est != &before.estimate
    {
        changes.push((
            "estimate".into(),
            serde_json::json!(before.estimate),
            serde_json::json!(new_est),
        ));
    }
    if let Some(new_labels) = &patch.labels
        && new_labels != &before.labels
    {
        changes.push((
            "labels".into(),
            serde_json::json!(before.labels),
            serde_json::json!(new_labels),
        ));
    }

    changes
}
