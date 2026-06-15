import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';

export type BoardDto = components['schemas']['BoardDto'];
export type ColumnDto = components['schemas']['ColumnDto'];
export type TaskSummaryDto = components['schemas']['TaskSummaryDto'];

/**
 * Reconcile shape: a subset of TaskDto fields we need to update a summary after a move.
 * The move endpoint returns a full TaskDto but we only need summary-compatible fields.
 */
export interface MovedTaskSummary {
  id: string;
  readable_id: string;
  column_id: string;
  title: string;
  priority: string | null | undefined;
  updated_at: string;
}

/**
 * Boards store: holds the active board, its columns (sorted by position_key),
 * and tasks per column (keyed by column_id, in server-returned order).
 *
 * Design Q7: boards store owns columns sorted by position_key and tasks grouped
 * per column sorted by fractional_index (server already returns them sorted).
 *
 * Provides optimistic-move primitives (snapshot/apply/restore) that useKanbanMove
 * composes without re-implementing the state shape.
 */
export const useBoardsStore = defineStore('boards', () => {
  const board = ref<BoardDto | null>(null);
  const columns = ref<ColumnDto[]>([]);
  const tasks = ref<Map<string, TaskSummaryDto[]>>(new Map());
  const loading = ref(false);
  const error = ref<string | null>(null);

  function tasksByColumn(columnId: string): TaskSummaryDto[] {
    return tasks.value.get(columnId) ?? [];
  }

  async function loadBoard(ws: string, boardId: string): Promise<void> {
    loading.value = true;
    error.value = null;

    const { data, error: apiError } = await wrappedClient.GET('/v1/workspaces/{ws}/boards/{board_id}', {
      params: { path: { ws, board_id: boardId } },
    });

    loading.value = false;

    if (apiError !== undefined || data === undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to load board';
      return;
    }

    board.value = data;
  }

  async function loadColumns(ws: string, boardId: string): Promise<void> {
    const { data, error: apiError } = await wrappedClient.GET(
      '/v1/workspaces/{ws}/boards/{board_id}/columns',
      { params: { path: { ws, board_id: boardId } } },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to load columns';
      return;
    }

    const raw = data as unknown as ColumnDto[];
    columns.value = [...raw].sort((a, b) => a.position_key.localeCompare(b.position_key));
  }

  async function loadTasks(ws: string, boardId: string): Promise<void> {
    const { data, error: apiError } = await wrappedClient.GET('/v1/workspaces/{ws}/boards/{board_id}/tasks', {
      params: { path: { ws, board_id: boardId } },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to load tasks';
      return;
    }

    const raw = data as unknown as {
      items: TaskSummaryDto[];
      has_more: boolean;
      next_cursor?: string | null;
    };
    const grouped = new Map<string, TaskSummaryDto[]>();

    for (const t of raw.items) {
      const col = grouped.get(t.column_id) ?? [];
      col.push(t);
      grouped.set(t.column_id, col);
    }

    tasks.value = grouped;
  }

  /**
   * Reconcile the task store after a successful move.
   * Removes the task from its old column, updates its fields (column_id, priority, etc.),
   * and appends it to the new column. The kanban composable may reorder after this.
   */
  function reconcileTask(moved: MovedTaskSummary): void {
    const newColumnId = moved.column_id;

    for (const [colId, colTasks] of tasks.value) {
      const idx = colTasks.findIndex((t) => t.id === moved.id);
      if (idx === -1) {
        continue;
      }

      tasks.value.set(
        colId,
        colTasks.filter((t) => t.id !== moved.id),
      );
      break;
    }

    const dest = tasks.value.get(newColumnId) ?? [];
    const updated: TaskSummaryDto = {
      id: moved.id,
      readable_id: moved.readable_id,
      column_id: newColumnId,
      title: moved.title,
      priority: moved.priority ?? null,
      updated_at: moved.updated_at,
    };
    tasks.value.set(newColumnId, [...dest, updated]);
  }

  /**
   * Optimistically move a task to a target column at the given index.
   * Removes the task from its current column and inserts it at `toIndex` in `toColumnId`.
   * The caller must have already taken a snapshot via snapshotTasks().
   */
  function applyOptimisticMove(taskId: string, toColumnId: string, toIndex: number): void {
    let movingTask: TaskSummaryDto | undefined;

    for (const [colId, colTasks] of tasks.value) {
      const idx = colTasks.findIndex((t) => t.id === taskId);
      if (idx === -1) {
        continue;
      }

      movingTask = colTasks[idx];
      tasks.value.set(
        colId,
        colTasks.filter((t) => t.id !== taskId),
      );
      break;
    }

    if (movingTask === undefined) {
      return;
    }

    const updated: TaskSummaryDto = { ...movingTask, column_id: toColumnId };
    const dest = [...(tasks.value.get(toColumnId) ?? [])];
    dest.splice(toIndex, 0, updated);
    tasks.value.set(toColumnId, dest);
  }

  /**
   * Snapshot the current per-column task ordering.
   * Returns a deep copy: mutations to the store after this call do not affect the snapshot.
   */
  function snapshotTasks(): Map<string, TaskSummaryDto[]> {
    const snap = new Map<string, TaskSummaryDto[]>();

    for (const [colId, colTasks] of tasks.value) {
      snap.set(
        colId,
        colTasks.map((t) => ({ ...t })),
      );
    }

    return snap;
  }

  /**
   * Restore tasks to a previously captured snapshot.
   * Used for rollback when a move fails.
   */
  function restoreSnapshot(snapshot: Map<string, TaskSummaryDto[]>): void {
    const restored = new Map<string, TaskSummaryDto[]>();

    for (const [colId, colTasks] of snapshot) {
      restored.set(
        colId,
        colTasks.map((t) => ({ ...t })),
      );
    }

    tasks.value = restored;
  }

  /**
   * Find a task by its readable_id across all columns.
   */
  function findTaskByReadableId(readableId: string): TaskSummaryDto | undefined {
    for (const [, colTasks] of tasks.value) {
      const found = colTasks.find((t) => t.readable_id === readableId);
      if (found !== undefined) {
        return found;
      }
    }
    return undefined;
  }

  /**
   * Update a task's metadata fields in-place within its current column.
   * Does not change which column the task is in — use applyOptimisticMove first.
   */
  function updateTaskFields(update: Partial<TaskSummaryDto> & { id: string }): void {
    for (const [colId, colTasks] of tasks.value) {
      const idx = colTasks.findIndex((t) => t.id === update.id);
      if (idx === -1) {
        continue;
      }

      const existing = colTasks[idx];
      if (existing === undefined) {
        break;
      }

      const updated: TaskSummaryDto = { ...existing, ...update };
      const newList = [...colTasks];
      newList[idx] = updated;
      tasks.value.set(colId, newList);
      break;
    }
  }

  /**
   * Replace the tasks array for a specific column.
   * Used by useKanbanMove to reorder after reconcile, and in tests.
   */
  function _setColumnTasks(columnId: string, colTasks: TaskSummaryDto[]): void {
    tasks.value.set(
      columnId,
      colTasks.map((t) => ({ ...t })),
    );
  }

  /**
   * Test helper: directly set the tasks map without going through the API.
   * Only call this from tests.
   */
  function _setTasksForTest(data: Record<string, TaskSummaryDto[]>): void {
    const m = new Map<string, TaskSummaryDto[]>();

    for (const [colId, colTasks] of Object.entries(data)) {
      m.set(
        colId,
        colTasks.map((t) => ({ ...t })),
      );
    }

    tasks.value = m;
  }

  return {
    board,
    columns,
    loading,
    error,
    tasksByColumn,
    loadBoard,
    loadColumns,
    loadTasks,
    reconcileTask,
    applyOptimisticMove,
    snapshotTasks,
    restoreSnapshot,
    findTaskByReadableId,
    updateTaskFields,
    _setColumnTasks,
    _setTasksForTest,
  };
});
