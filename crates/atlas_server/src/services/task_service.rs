use atlas_domain::{
    DomainError, WorkspaceCtx,
    entities::boards_tasks::{
        ActivityKind, ActivityPayload, AssigneeRef, NewTask, NewTaskActivity, NewTaskAssignee,
        NewTaskChecklistItem, NewTaskReference, PositionBetween, ReferenceKind, Task, TaskActivity,
        TaskAssignee, TaskChecklistItem, TaskChecklistItemPatch, TaskPatch, TaskReference,
    },
    ids::{BoardId, ChecklistItemId, ColumnId, ProjectId, TaskActivityId, TaskId, TaskReferenceId},
};
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QuerySelect, TransactionTrait,
};

use atlas_domain::entities::documents::ExtractedLink;
use atlas_domain::{parse_wikilinks, slugify};
use sea_orm::ConnectionTrait;

use crate::persistence::entities::boards_tasks::{
    board, board_column, task, task_checklist_item, task_checklist_item_from,
};
use crate::persistence::repos::{
    PgDocumentLinkRepo, PgTaskActivityRepo, PgTaskAssigneeRepo, PgTaskChecklistRepo,
    PgTaskReferenceRepo, PgTaskRepo, TaskActivityRepo as _,
};

/// Result of a checklist item promotion: the three records committed atomically.
pub struct PromotionResult {
    pub task: Task,
    pub parent_reference: TaskReference,
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
    pub fn new(conn: DatabaseConnection) -> Self {
        Self { conn }
    }

    /// Creates a task and appends a `Created` activity in the same transaction.
    pub async fn create(&self, ctx: &WorkspaceCtx, new: NewTask) -> Result<Task, DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;

        let target_board_id = new.board_id;
        let target_column_id = new.column_id;
        let target_project_id = new.project_id;
        validate_column_in_board(
            &txn,
            ctx,
            target_board_id,
            target_column_id,
            target_project_id,
        )
        .await?;

        let task = PgTaskRepo::create_in(&txn, ctx, new).await?;

        sync_task_description_links(&txn, ctx, task.id, &task.description).await?;

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

        let description_changed = patch.description.is_some();

        let updated = PgTaskRepo::patch_in(&txn, ctx, id, patch).await?;

        if description_changed {
            sync_task_description_links(&txn, ctx, id, &updated.description).await?;
        }

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

        // A task may only move between columns of its own board; the target
        // column must belong to that board and the caller's workspace.
        validate_column_in_board(&txn, ctx, before.board_id, column_id, before.project_id).await?;

        // Clients send neighbour anchors as task ids; translate them to the
        // neighbours' fractional position keys before computing the new key.
        let resolved = PositionBetween {
            before: resolve_anchor_key(&txn, ctx, position.before.as_deref()).await?,
            after: resolve_anchor_key(&txn, ctx, position.after.as_deref()).await?,
        };

        let moved = PgTaskRepo::move_to_in(&txn, ctx, id, column_id, resolved).await?;

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
    ///
    /// Loads the stored reference BEFORE deleting it so the activity payload
    /// carries the real `kind`. The scoped load (`source_task_id = task_id`)
    /// doubles as the existence + ownership check, so a missing or cross-task
    /// reference produces `NotFound` without a separate round-trip.
    pub async fn remove_reference(
        &self,
        ctx: &WorkspaceCtx,
        task_id: TaskId,
        reference_id: TaskReferenceId,
    ) -> Result<(), DomainError> {
        use crate::persistence::entities::boards_tasks::{task_reference, task_reference_from};

        let txn = self.conn.begin().await.map_err(db_err)?;

        let row = task_reference::Entity::find_by_id(reference_id.0)
            .filter(task_reference::Column::WorkspaceId.eq(ctx.workspace_id.0))
            .filter(task_reference::Column::SourceTaskId.eq(task_id.0))
            .one(&txn)
            .await
            .map_err(db_err)?
            .ok_or(DomainError::NotFound {
                entity: "task_reference",
                id: reference_id.0,
            })?;

        let stored = task_reference_from(row).map_err(|m| DomainError::Internal { message: m })?;
        let kind = stored.kind.clone();

        PgTaskReferenceRepo::delete_in(&txn, ctx, task_id, reference_id).await?;

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

        let item = PgTaskChecklistRepo::patch_item_in(&txn, ctx, task_id, item_id, patch).await?;

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

        PgTaskChecklistRepo::soft_delete_item_in(&txn, ctx, task_id, item_id).await?;

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
        parent_task_id: TaskId,
        item_id: ChecklistItemId,
        project_id: ProjectId,
        board_id: BoardId,
        column_id: ColumnId,
    ) -> Result<PromotionResult, DomainError> {
        let txn = self.conn.begin().await.map_err(db_err)?;

        // Fetch the item directly by ID so we can read its parent task_id and
        // guard against re-promotion, all within the open transaction. The item
        // must belong to the authorized parent task (intra-workspace IDOR guard).
        let item = {
            task_checklist_item::Entity::find_by_id(item_id.0)
                .filter(task_checklist_item::Column::WorkspaceId.eq(ctx.workspace_id.0))
                .filter(task_checklist_item::Column::TaskId.eq(parent_task_id.0))
                .filter(task_checklist_item::Column::DeletedAt.is_null())
                .lock_exclusive()
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

        // The target board+column come from the request body, so verify they
        // live in the caller's workspace, belong to the expected project, and
        // that the column belongs to the target board before writing a task.
        validate_column_in_board(&txn, ctx, board_id, column_id, project_id).await?;

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
        .await?;

        let updated_item = PgTaskChecklistRepo::mark_promoted_in(
            &txn,
            ctx,
            parent_task_id,
            item_id,
            child_task.id,
        )
        .await?;

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

/// Verifies that `board_id` lives in the caller's workspace, that the board
/// belongs to `project_id`, and that `column_id` is a live column of that board.
///
/// The project_id check is defense-in-depth: the boards table has no composite
/// FK on (board_id, project_id), so without this assertion a future caller path
/// that regresses on the board constraint could silently write a task row that
/// is inconsistent between its board and its project.
///
/// Returns `NotFound` for an unknown board, and `InvalidInput` (422) when the
/// board's project does not match or the column does not belong to the board.
/// Resolves a move anchor — a task id, as sent by clients — to that task's
/// fractional position key. Returns `None` when the anchor is absent, not a UUID,
/// or refers to no live task, so the move falls back to a boundary/default
/// placement instead of failing. Fractional keys stay server-internal: clients
/// reference neighbours by task id, never by raw position key.
async fn resolve_anchor_key(
    conn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
    anchor: Option<&str>,
) -> Result<Option<String>, DomainError> {
    let Some(raw) = anchor else {
        return Ok(None);
    };
    let Ok(task_id) = uuid::Uuid::parse_str(raw) else {
        return Ok(None);
    };

    let row = task::Entity::find_by_id(task_id)
        .filter(task::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(task::Column::DeletedAt.is_null())
        .one(conn)
        .await
        .map_err(db_err)?;

    Ok(row.map(|r| r.position_key))
}

async fn validate_column_in_board(
    conn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
    board_id: BoardId,
    column_id: ColumnId,
    project_id: ProjectId,
) -> Result<(), DomainError> {
    let board = board::Entity::find_by_id(board_id.0)
        .filter(board::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(board::Column::DeletedAt.is_null())
        .one(conn)
        .await
        .map_err(db_err)?
        .ok_or(DomainError::NotFound {
            entity: "board",
            id: board_id.0,
        })?;

    if board.project_id != project_id.0 {
        return Err(DomainError::InvalidInput {
            message: "board does not belong to the expected project".into(),
        });
    }

    let column = board_column::Entity::find_by_id(column_id.0)
        .filter(board_column::Column::WorkspaceId.eq(ctx.workspace_id.0))
        .filter(board_column::Column::DeletedAt.is_null())
        .one(conn)
        .await
        .map_err(db_err)?;

    match column {
        Some(c) if c.board_id == board_id.0 => Ok(()),
        _ => Err(DomainError::InvalidInput {
            message: "column does not belong to the target board".into(),
        }),
    }
}

/// Parses wikilinks out of a task description, resolves each `[[Title]]` to a
/// live document by slug, and replaces the task's link set — all on `conn`, so
/// it joins the caller's transaction.
///
/// Mirrors E04's document wikilink flow: a title that resolves to no live
/// document is stored as a pending link (target_document_id NULL), not dropped.
/// This pending-link behavior applies to task description wikilinks (`document_links`)
/// only; typed `task_references` reject non-existent targets outright.
async fn sync_task_description_links(
    conn: &impl ConnectionTrait,
    ctx: &WorkspaceCtx,
    task_id: TaskId,
    description: &str,
) -> Result<(), DomainError> {
    let titles = parse_wikilinks(description);

    let mut extracted = Vec::with_capacity(titles.len());
    for title in titles {
        let target_document_id =
            PgDocumentLinkRepo::find_document_id_by_slug_in(conn, ctx, &slugify(&title)).await?;

        extracted.push(ExtractedLink {
            target_title: title,
            target_document_id,
        });
    }

    PgDocumentLinkRepo::replace_for_task_source_in(conn, ctx, task_id, extracted).await
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
