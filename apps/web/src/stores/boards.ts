import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';

export type BoardDto = components['schemas']['BoardDto'];
export type BoardSummaryDto = components['schemas']['BoardSummaryDto'];
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
  const boardSummaries = ref<BoardSummaryDto[]>([]);
  // Board lists keyed by project slug, for sidebars that list every project.
  const boardsByProject = ref<Map<string, BoardSummaryDto[]>>(new Map());
  const columns = ref<ColumnDto[]>([]);
  const tasks = ref<Map<string, TaskSummaryDto[]>>(new Map());
  const loading = ref(false);
  const error = ref<string | null>(null);

  // Stable empty-array reference per column. Returning a fresh `[]` for an empty
  // column on every call makes the kanban draggable's bound list change identity
  // on every render, which drives an infinite render loop (renderer freeze).
  const emptyByColumn = new Map<string, TaskSummaryDto[]>();

  function tasksByColumn(columnId: string): TaskSummaryDto[] {
    const existing = tasks.value.get(columnId);
    if (existing !== undefined) {
      return existing;
    }

    let empty = emptyByColumn.get(columnId);
    if (empty === undefined) {
      empty = [];
      emptyByColumn.set(columnId, empty);
    }
    return empty;
  }

  async function loadBoards(ws: string, projectSlug: string): Promise<void> {
    const { data, error: apiError } = await wrappedClient.GET(
      '/v1/workspaces/{ws}/projects/{project_slug}/boards',
      { params: { path: { ws, project_slug: projectSlug } } },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to load boards';
      return;
    }

    boardSummaries.value = data.items;
  }

  async function loadBoardsForProject(ws: string, projectSlug: string): Promise<void> {
    const { data, error: apiError } = await wrappedClient.GET(
      '/v1/workspaces/{ws}/projects/{project_slug}/boards',
      { params: { path: { ws, project_slug: projectSlug } } },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to load boards';
      return;
    }

    const next = new Map(boardsByProject.value);
    next.set(projectSlug, data.items);
    boardsByProject.value = next;
  }

  function boardsFor(projectSlug: string): BoardSummaryDto[] {
    return boardsByProject.value.get(projectSlug) ?? [];
  }

  async function createBoard(ws: string, projectSlug: string, name: string): Promise<string | null> {
    const { data, error: apiError } = await wrappedClient.POST(
      '/v1/workspaces/{ws}/projects/{project_slug}/boards',
      { params: { path: { ws, project_slug: projectSlug } }, body: { name } },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to create board';
      return null;
    }

    await loadBoardsForProject(ws, projectSlug);
    return data.id ?? null;
  }

  async function renameBoard(
    ws: string,
    projectSlug: string,
    boardId: string,
    name: string,
  ): Promise<boolean> {
    const { error: apiError } = await wrappedClient.PATCH('/v1/workspaces/{ws}/boards/{board_id}', {
      params: { path: { ws, board_id: boardId } },
      body: { name },
    });

    if (apiError !== undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to rename board';
      return false;
    }

    await loadBoardsForProject(ws, projectSlug);
    return true;
  }

  async function removeBoard(ws: string, projectSlug: string, boardId: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.DELETE('/v1/workspaces/{ws}/boards/{board_id}', {
      params: { path: { ws, board_id: boardId } },
    });

    if (apiError !== undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to delete board';
      return false;
    }

    await loadBoardsForProject(ws, projectSlug);
    return true;
  }

  async function createTask(
    ws: string,
    boardId: string,
    columnId: string,
    title: string,
  ): Promise<string | null> {
    const { data, error: apiError } = await wrappedClient.POST(
      '/v1/workspaces/{ws}/boards/{board_id}/tasks',
      { params: { path: { ws, board_id: boardId } }, body: { column_id: columnId, title } },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to create task';
      return null;
    }

    await loadTasks(ws, boardId);
    return data.readable_id ?? null;
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

    columns.value = [...data].sort((a, b) => a.position_key.localeCompare(b.position_key));
  }

  async function loadTasks(ws: string, boardId: string): Promise<void> {
    const { data, error: apiError } = await wrappedClient.GET('/v1/workspaces/{ws}/boards/{board_id}/tasks', {
      params: { path: { ws, board_id: boardId } },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to load tasks';
      return;
    }

    const grouped = new Map<string, TaskSummaryDto[]>();

    for (const t of data.items) {
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

  function removeTaskById(taskId: string): void {
    for (const [colId, colTasks] of tasks.value) {
      if (colTasks.some((t) => t.id === taskId)) {
        tasks.value.set(
          colId,
          colTasks.filter((t) => t.id !== taskId),
        );
        break;
      }
    }
  }

  /**
   * Patch a task's editable fields (title, priority, due date) and reflect the
   * title/priority change in the local summary. `null` clears priority/due date;
   * `undefined` leaves a field untouched.
   */
  async function updateTask(
    ws: string,
    readableId: string,
    patch: {
      title?: string;
      priority?: string | null;
      due_date?: string | null;
      estimate?: number | null;
      labels?: string[];
    },
  ): Promise<boolean> {
    const { data, error: apiError } = await wrappedClient.PATCH('/v1/workspaces/{ws}/tasks/{readable_id}', {
      params: { path: { ws, readable_id: readableId } },
      body: patch,
    });

    if (apiError !== undefined || data === undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to update task';
      return false;
    }

    const summaryPatch: Partial<TaskSummaryDto> & { id: string } = { id: data.id };
    if (patch.title !== undefined) summaryPatch.title = data.title;
    if (patch.priority !== undefined) summaryPatch.priority = data.priority ?? null;
    updateTaskFields(summaryPatch);

    return true;
  }

  async function deleteTask(ws: string, readableId: string): Promise<boolean> {
    const target = findTaskByReadableId(readableId);

    const { error: apiError } = await wrappedClient.DELETE('/v1/workspaces/{ws}/tasks/{readable_id}', {
      params: { path: { ws, readable_id: readableId } },
    });

    if (apiError !== undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to delete task';
      return false;
    }

    if (target !== undefined) removeTaskById(target.id);
    return true;
  }

  /** Assigns a workspace member (user) or agent (api_key) to a task. */
  async function assignTask(
    ws: string,
    readableId: string,
    principalType: string,
    principalId: string,
  ): Promise<boolean> {
    const { error: apiError } = await wrappedClient.POST(
      '/v1/workspaces/{ws}/tasks/{readable_id}/assignees',
      {
        params: { path: { ws, readable_id: readableId } },
        body: { assignee_type: principalType, assignee_id: principalId },
      },
    );

    if (apiError !== undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to assign task';
      return false;
    }

    return true;
  }

  /**
   * Duplicates a task into the same column on the active board, copying its title
   * (suffixed " (copy)"), description and priority. The create endpoint takes no
   * priority, so a follow-up patch sets it when the source had one. Returns the
   * new task's readable id, or null on failure.
   */
  async function duplicateTask(ws: string, boardId: string, readableId: string): Promise<string | null> {
    const { data: source, error: getErr } = await wrappedClient.GET(
      '/v1/workspaces/{ws}/tasks/{readable_id}',
      { params: { path: { ws, readable_id: readableId } } },
    );

    if (getErr !== undefined || source === undefined) {
      error.value = (getErr as { hint?: string } | undefined)?.hint ?? 'Failed to read task';
      return null;
    }

    const { data: created, error: createErr } = await wrappedClient.POST(
      '/v1/workspaces/{ws}/boards/{board_id}/tasks',
      {
        params: { path: { ws, board_id: boardId } },
        body: {
          column_id: source.column_id,
          title: `${source.title} (copy)`,
          description: source.description,
        },
      },
    );

    if (createErr !== undefined || created === undefined) {
      error.value = (createErr as { hint?: string } | undefined)?.hint ?? 'Failed to duplicate task';
      return null;
    }

    if (source.priority !== undefined && source.priority !== null) {
      await wrappedClient.PATCH('/v1/workspaces/{ws}/tasks/{readable_id}', {
        params: { path: { ws, readable_id: created.readable_id } },
        body: { priority: source.priority },
      });
    }

    await loadTasks(ws, boardId);
    return created.readable_id ?? null;
  }

  /**
   * Moves a task to a column (status). The column may live on a different board:
   * the server adopts the target board/project. Reloads the active board so the
   * task lands in its new column or disappears when it left the board entirely.
   */
  async function moveTaskToColumn(ws: string, readableId: string, columnId: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.POST('/v1/workspaces/{ws}/tasks/{readable_id}/move', {
      params: { path: { ws, readable_id: readableId } },
      body: { column_id: columnId, before: null, after: null },
    });

    if (apiError !== undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to move task';
      return false;
    }

    const activeBoardId = board.value?.id;
    if (activeBoardId !== undefined) await loadTasks(ws, activeBoardId);
    return true;
  }

  /**
   * Moves a task to another board, landing it in that board's first column.
   * Returns false (with `error` set) when the target board has no columns.
   */
  async function moveTaskToBoard(ws: string, readableId: string, targetBoardId: string): Promise<boolean> {
    const { data, error: apiError } = await wrappedClient.GET(
      '/v1/workspaces/{ws}/boards/{board_id}/columns',
      { params: { path: { ws, board_id: targetBoardId } } },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to read target board';
      return false;
    }

    const sorted = [...data].sort((a, b) => a.position_key.localeCompare(b.position_key));
    const first = sorted[0];
    if (first === undefined) {
      error.value = 'The target board has no columns to move into';
      return false;
    }

    return moveTaskToColumn(ws, readableId, first.id);
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
    boardSummaries,
    columns,
    loading,
    error,
    tasksByColumn,
    loadBoards,
    loadBoardsForProject,
    boardsFor,
    createBoard,
    renameBoard,
    removeBoard,
    createTask,
    loadBoard,
    loadColumns,
    loadTasks,
    reconcileTask,
    applyOptimisticMove,
    snapshotTasks,
    restoreSnapshot,
    findTaskByReadableId,
    updateTaskFields,
    updateTask,
    deleteTask,
    assignTask,
    duplicateTask,
    moveTaskToColumn,
    moveTaskToBoard,
    _setColumnTasks,
    _setTasksForTest,
  };
});
