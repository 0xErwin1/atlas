import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import { errorHint } from '@/lib/apiError';
import { collectPaged } from '@/lib/pagination';
import { useLabelColorsStore } from '@/stores/labelColors';
import { useUiStore } from '@/stores/ui';

export type BoardDto = components['schemas']['BoardDto'];
export type BoardSummaryDto = components['schemas']['BoardSummaryDto'];
export type ColumnDto = components['schemas']['ColumnDto'];
export type TaskSummaryDto = components['schemas']['TaskSummaryDto'];
export type ActorDto = components['schemas']['ActorDto'];
export type TaskDto = components['schemas']['TaskDto'];

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
  // Full task DTOs keyed by readable_id, lazily fetched for the date-driven
  // layouts (calendar, timeline, table Due column) since the bulk task summary
  // carries no due_date. The data model has no start_date at all.
  const taskDetails = ref<Map<string, TaskDto>>(new Map());
  const detailsLoading = ref(false);
  const loading = ref(false);
  // Two distinct error channels. `loadError` reflects a failure of the board's
  // own load path (board / columns / tasks GETs) and is the only signal that
  // gates the full-screen "Couldn't load board" panel. `error` is the transient
  // action channel (assign, move, delete, …) surfaced as toasts; a failed action
  // must never blank an already-loaded board.
  const loadError = ref<string | null>(null);
  const error = ref<string | null>(null);
  interface ActiveBoardLoad {
    ws: string;
    boardId: string;
    refreshColumns: boolean;
  }

  let activeBoardLoad: ActiveBoardLoad | null = null;
  let activeBoardRequest: object | null = null;
  let activeColumnsRequest: object | null = null;
  let activeTasksRequest: object | null = null;
  let activeTaskDetailsRequest: object | null = null;

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

  // Memoized filtered lists keyed by column. Identity must stay stable across
  // renders while a column's raw tasks and the active filter are unchanged: the
  // kanban draggable freezes in an infinite render loop if its bound list
  // changes identity on every render (see emptyByColumn above). Safe because the
  // raw arrays are always replaced, never mutated in place, so `raw` reference
  // equality is a sound cache-invalidation signal.
  const filteredByColumn = new Map<
    string,
    { raw: TaskSummaryDto[]; key: string; result: TaskSummaryDto[] }
  >();

  /**
   * Returns the tasks for a column after applying the active filter from useUiStore.
   *
   * Semantics: a task passes a dimension when that dimension's selected array is
   * empty (inactive) OR the task matches at least one value in it (OR within
   * dimension). A task is included only when it passes ALL four dimensions (AND
   * across dimensions).
   *
   * Dimension → TaskSummaryDto field mapping:
   *   statuses    → column_id
   *   priorities  → priority
   *   labels      → labels[] (any label must be in the selected set)
   *   assigneeIds → assignees[].id (any assignee id must be in the selected set)
   */
  function filteredTasksByColumn(columnId: string): TaskSummaryDto[] {
    const raw = tasksByColumn(columnId);
    const ui = useUiStore();
    const filter = ui.taskFilter;
    const text = ui.taskFilterText.trim().toLowerCase();

    const noStatusFilter = filter.statuses.length === 0;
    const noPriorityFilter = filter.priorities.length === 0;
    const noLabelFilter = filter.labels.length === 0;
    const noAssigneeFilter = filter.assigneeIds.length === 0;
    const noTextFilter = text === '';

    if (noStatusFilter && noPriorityFilter && noLabelFilter && noAssigneeFilter && noTextFilter) {
      return raw;
    }

    const key = `${filter.statuses.join(',')}|${filter.priorities.join(',')}|${filter.labels.join(',')}|${filter.assigneeIds.join(',')}|${text}`;
    const cached = filteredByColumn.get(columnId);
    if (cached !== undefined && cached.raw === raw && cached.key === key) {
      return cached.result;
    }

    const statusSet = new Set(filter.statuses);
    const prioritySet = new Set(filter.priorities);
    const labelSet = new Set(filter.labels);
    const assigneeSet = new Set(filter.assigneeIds);

    const result = raw.filter((task) => {
      if (!noStatusFilter && !statusSet.has(task.column_id)) return false;
      if (!noPriorityFilter && !prioritySet.has(task.priority ?? '')) return false;
      if (!noLabelFilter && !(task.labels ?? []).some((l) => labelSet.has(l))) return false;
      if (!noAssigneeFilter && !(task.assignees ?? []).some((a) => assigneeSet.has(a.id))) return false;
      if (
        !noTextFilter &&
        !task.title.toLowerCase().includes(text) &&
        !task.readable_id.toLowerCase().includes(text)
      ) {
        return false;
      }

      return true;
    });

    filteredByColumn.set(columnId, { raw, key, result });
    return result;
  }

  async function loadBoards(ws: string, projectSlug: string): Promise<void> {
    const { items, error: apiError } = await collectPaged<BoardSummaryDto>((cursor) =>
      wrappedClient.GET('/api/workspaces/{ws}/projects/{project_slug}/boards', {
        params: {
          path: { ws, project_slug: projectSlug },
          query: { limit: 200, ...(cursor !== undefined ? { cursor } : {}) },
        },
      }),
    );

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to load boards');
      return;
    }

    boardSummaries.value = items;
  }

  async function loadBoardsForProject(ws: string, projectSlug: string): Promise<void> {
    const { items, error: apiError } = await collectPaged<BoardSummaryDto>((cursor) =>
      wrappedClient.GET('/api/workspaces/{ws}/projects/{project_slug}/boards', {
        params: {
          path: { ws, project_slug: projectSlug },
          query: { limit: 200, ...(cursor !== undefined ? { cursor } : {}) },
        },
      }),
    );

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to load boards');
      return;
    }

    const next = new Map(boardsByProject.value);
    next.set(projectSlug, items);
    boardsByProject.value = next;
  }

  function boardsFor(projectSlug: string): BoardSummaryDto[] {
    return boardsByProject.value.get(projectSlug) ?? [];
  }

  async function createBoard(ws: string, projectSlug: string, name: string): Promise<string | null> {
    const { data, error: apiError } = await wrappedClient.POST(
      '/api/workspaces/{ws}/projects/{project_slug}/boards',
      { params: { path: { ws, project_slug: projectSlug } }, body: { name } },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to create board');
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
    const { error: apiError } = await wrappedClient.PATCH('/api/workspaces/{ws}/boards/{board_id}', {
      params: { path: { ws, board_id: boardId } },
      body: { name },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to rename board');
      return false;
    }

    await loadBoardsForProject(ws, projectSlug);
    return true;
  }

  async function removeBoard(ws: string, projectSlug: string, boardId: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.DELETE('/api/workspaces/{ws}/boards/{board_id}', {
      params: { path: { ws, board_id: boardId } },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to delete board');
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
      '/api/workspaces/{ws}/boards/{board_id}/tasks',
      { params: { path: { ws, board_id: boardId } }, body: { column_id: columnId, title } },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to create task');
      return null;
    }

    await loadTasks(ws, boardId);
    return data.readable_id ?? null;
  }

  function fetchBoard(ws: string, boardId: string) {
    return wrappedClient.GET('/api/workspaces/{ws}/boards/{board_id}', {
      params: { path: { ws, board_id: boardId } },
    });
  }

  function fetchColumns(ws: string, boardId: string) {
    return wrappedClient.GET('/api/workspaces/{ws}/boards/{board_id}/columns', {
      params: { path: { ws, board_id: boardId } },
    });
  }

  function fetchTasks(ws: string, boardId: string) {
    return collectPaged<TaskSummaryDto>((cursor) =>
      wrappedClient.GET('/api/workspaces/{ws}/boards/{board_id}/tasks', {
        params: {
          path: { ws, board_id: boardId },
          query: { limit: 200, ...(cursor !== undefined ? { cursor } : {}) },
        },
      }),
    );
  }

  function groupTasks(items: TaskSummaryDto[]): Map<string, TaskSummaryDto[]> {
    const grouped = new Map<string, TaskSummaryDto[]>();

    for (const task of items) {
      const columnTasks = grouped.get(task.column_id) ?? [];
      columnTasks.push(task);
      grouped.set(task.column_id, columnTasks);
    }

    return grouped;
  }

  type SettledRequest<T> = { value: T } | { cause: unknown };

  async function settleRequest<T>(promise: Promise<T>): Promise<SettledRequest<T>> {
    try {
      return { value: await promise };
    } catch (cause) {
      return { cause };
    }
  }

  function settledError<T extends { error?: unknown }>(
    result: SettledRequest<T>,
    fallback: string,
    isValid: (value: T) => boolean = () => true,
  ): string | null {
    if ('cause' in result) return errorHint(result.cause, fallback);
    if (result.value.error !== undefined || !isValid(result.value)) {
      return errorHint(result.value.error, fallback);
    }
    return null;
  }

  async function loadBoard(ws: string, boardId: string): Promise<void> {
    if (activeBoardLoad !== null) return;

    const request = {};
    activeBoardRequest = request;
    loading.value = true;
    loadError.value = null;
    error.value = null;

    const result = await settleRequest(fetchBoard(ws, boardId));

    if (activeBoardRequest !== request || activeBoardLoad !== null) return;

    activeBoardRequest = null;
    loading.value = false;

    const nextError = settledError(result, 'Failed to load board', (value) => value.data !== undefined);
    if (nextError !== null || !('value' in result) || result.value.data === undefined) {
      loadError.value = nextError ?? 'Failed to load board';
      return;
    }

    board.value = result.value.data;
  }

  async function loadColumns(ws: string, boardId: string): Promise<void> {
    if (activeBoardLoad !== null) {
      if (activeBoardLoad.ws === ws && activeBoardLoad.boardId === boardId) {
        activeBoardLoad.refreshColumns = true;
      }
      return;
    }

    const request = {};
    activeColumnsRequest = request;

    const result = await settleRequest(fetchColumns(ws, boardId));

    if (activeColumnsRequest !== request || activeBoardLoad !== null) return;

    activeColumnsRequest = null;
    const nextError = settledError(result, 'Failed to load columns', (value) => value.data !== undefined);
    if (nextError !== null || !('value' in result) || result.value.data === undefined) {
      loadError.value = nextError ?? 'Failed to load columns';
      return;
    }

    columns.value = [...result.value.data].sort((a, b) => a.position_key.localeCompare(b.position_key));
  }

  /**
   * Creates a new column (status) appended after the current last one on the
   * board, then inserts it into the sorted `columns` list. Columns order by
   * `position_key`, so the new key is requested between the last column and the
   * end (`before` = last key, no `after`).
   */
  async function createColumn(ws: string, boardId: string, name: string): Promise<ColumnDto | null> {
    const last = columns.value.at(-1);

    const { data, error: apiError } = await wrappedClient.POST(
      '/api/workspaces/{ws}/boards/{board_id}/columns',
      {
        params: { path: { ws, board_id: boardId } },
        body: { name, before: last?.position_key ?? null, after: null },
      },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to create status');
      return null;
    }

    columns.value = [...columns.value, data].sort((a, b) => a.position_key.localeCompare(b.position_key));
    return data;
  }

  /**
   * Patches a column's name and/or color (color is a swatch id; `null` clears it
   * back to the deterministic default). Replaces the cached column in place,
   * re-sorting so a name change never disturbs the ordering. Returns true on
   * success; sets `error` otherwise.
   */
  async function updateColumn(
    ws: string,
    boardId: string,
    columnId: string,
    patch: { name?: string; color?: string | null },
  ): Promise<boolean> {
    const { data, error: apiError } = await wrappedClient.PATCH(
      '/api/workspaces/{ws}/boards/{board_id}/columns/{column_id}',
      {
        params: { path: { ws, board_id: boardId, column_id: columnId } },
        body: patch,
      },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to update status');
      return false;
    }

    columns.value = columns.value
      .map((c) => (c.id === columnId ? data : c))
      .sort((a, b) => a.position_key.localeCompare(b.position_key));
    return true;
  }

  /**
   * Reorders a column by requesting a new position between `before`/`after`
   * sibling position keys. Re-sorts the cache by the returned `position_key`.
   * Returns true on success; sets `error` otherwise.
   */
  async function moveColumn(
    ws: string,
    boardId: string,
    columnId: string,
    placement: { before: string | null; after: string | null },
  ): Promise<boolean> {
    const { data, error: apiError } = await wrappedClient.PATCH(
      '/api/workspaces/{ws}/boards/{board_id}/columns/{column_id}',
      {
        params: { path: { ws, board_id: boardId, column_id: columnId } },
        body: { before: placement.before, after: placement.after },
      },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to reorder status');
      return false;
    }

    columns.value = columns.value
      .map((c) => (c.id === columnId ? data : c))
      .sort((a, b) => a.position_key.localeCompare(b.position_key));
    return true;
  }

  /** Deletes a column (status) and drops it from the cache. Returns true on success. */
  async function deleteColumn(ws: string, boardId: string, columnId: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.DELETE(
      '/api/workspaces/{ws}/boards/{board_id}/columns/{column_id}',
      { params: { path: { ws, board_id: boardId, column_id: columnId } } },
    );

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to delete status');
      return false;
    }

    columns.value = columns.value.filter((c) => c.id !== columnId);
    return true;
  }

  async function loadTasks(ws: string, boardId: string): Promise<void> {
    if (activeBoardLoad !== null) return;

    const request = {};
    activeTasksRequest = request;

    const result = await settleRequest(fetchTasks(ws, boardId));

    if (activeTasksRequest !== request || activeBoardLoad !== null) return;

    activeTasksRequest = null;
    const nextError = settledError(result, 'Failed to load tasks');
    if (nextError !== null || !('value' in result)) {
      loadError.value = nextError ?? 'Failed to load tasks';
      return;
    }

    tasks.value = groupTasks(result.value.items);

    // Feed the (endpoint-less) tag registry from real labels so facets can offer them.
    useLabelColorsStore().recordTags(result.value.items.flatMap((task) => task.labels ?? []));
  }

  async function loadBoardContents(ws: string, boardId: string): Promise<boolean> {
    const operation: ActiveBoardLoad = { ws, boardId, refreshColumns: false };
    activeBoardLoad = operation;
    activeBoardRequest = null;
    activeColumnsRequest = null;
    activeTasksRequest = null;
    invalidateTaskDetails();

    loading.value = true;
    loadError.value = null;
    error.value = null;
    board.value = null;
    columns.value = [];
    tasks.value = new Map();

    const [boardResult, initialColumnsResult, tasksResult] = await Promise.all([
      settleRequest(fetchBoard(ws, boardId)),
      settleRequest(fetchColumns(ws, boardId)),
      settleRequest(fetchTasks(ws, boardId)),
    ]);

    if (activeBoardLoad !== operation) return false;

    let columnsResult = initialColumnsResult;
    while (operation.refreshColumns) {
      operation.refreshColumns = false;
      columnsResult = await settleRequest(fetchColumns(ws, boardId));
      if (activeBoardLoad !== operation) return false;
    }

    const boardError = settledError(boardResult, 'Failed to load board', (value) => value.data !== undefined);
    const columnsError = settledError(
      columnsResult,
      'Failed to load columns',
      (value) => value.data !== undefined,
    );
    const tasksError = settledError(tasksResult, 'Failed to load tasks');

    activeBoardLoad = null;
    loading.value = false;

    const nextError = boardError ?? columnsError ?? tasksError;
    if (nextError !== null) {
      loadError.value = nextError;
      return true;
    }

    if (
      !('value' in boardResult) ||
      boardResult.value.data === undefined ||
      !('value' in columnsResult) ||
      columnsResult.value.data === undefined ||
      !('value' in tasksResult)
    ) {
      loadError.value = 'Failed to load board';
      return true;
    }

    board.value = boardResult.value.data;
    columns.value = [...columnsResult.value.data].sort((a, b) =>
      a.position_key.localeCompare(b.position_key),
    );
    tasks.value = groupTasks(tasksResult.value.items);
    useLabelColorsStore().recordTags(tasksResult.value.items.flatMap((task) => task.labels ?? []));

    return true;
  }

  function cancelBoardLoad(): void {
    activeBoardLoad = null;
    activeBoardRequest = null;
    activeColumnsRequest = null;
    activeTasksRequest = null;
    loading.value = false;
    loadError.value = null;
    invalidateTaskDetails();
  }

  function invalidateTaskDetails(): void {
    activeTaskDetailsRequest = null;
    taskDetails.value = new Map();
    detailsLoading.value = false;
  }

  /**
   * Insert-or-update a single task's card on the active board from a live event
   * that carries only its id. The per-task summary is not addressable by UUID
   * (the task GET route resolves a readable id), so the board's task list is
   * refetched and only the affected column(s) are rebuilt from it, leaving every
   * other column's array identity untouched — a background update must not churn
   * the whole kanban. Idempotent: rebuilding the touched columns from the
   * authoritative list yields the same result when the event echoes a change
   * this client already applied, so a card is never duplicated. When the task is
   * no longer on the board (moved off or deleted) any stale copy is removed.
   */
  async function upsertTaskById(ws: string, taskId: string): Promise<void> {
    const boardId = board.value?.id;
    if (boardId === undefined) return;

    const { items, error: apiError } = await collectPaged<TaskSummaryDto>((cursor) =>
      wrappedClient.GET('/api/workspaces/{ws}/boards/{board_id}/tasks', {
        params: {
          path: { ws, board_id: boardId },
          query: { limit: 200, ...(cursor !== undefined ? { cursor } : {}) },
        },
      }),
    );

    if (apiError !== undefined) return;

    const summary = items.find((t) => t.id === taskId);

    if (summary === undefined) {
      removeTaskById(taskId);
      return;
    }

    let staleColumn: string | undefined;
    for (const [colId, colTasks] of tasks.value) {
      if (colTasks.some((t) => t.id === taskId)) {
        staleColumn = colId;
        break;
      }
    }

    _setColumnTasks(
      summary.column_id,
      items.filter((t) => t.column_id === summary.column_id),
    );

    if (staleColumn !== undefined && staleColumn !== summary.column_id) {
      _setColumnTasks(
        staleColumn,
        items.filter((t) => t.column_id === staleColumn),
      );
    }

    useLabelColorsStore().recordTags(summary.labels ?? []);
  }

  function taskDetail(readableId: string): TaskDto | undefined {
    return taskDetails.value.get(readableId);
  }

  /**
   * Fetch full DTOs for every task currently on the board so the date-driven
   * layouts have real due dates. The bulk summary endpoint omits due_date, so
   * this is a per-task fan-out; boards are small enough that one parallel batch
   * is acceptable. Never fabricates dates — a task with no due_date stays absent.
   */
  async function loadTaskDetails(ws: string): Promise<void> {
    const ids = [...tasks.value.values()].flat().map((t) => t.readable_id);
    const request = {};
    activeTaskDetailsRequest = request;

    detailsLoading.value = true;

    const result = await settleRequest(
      Promise.all(
        ids.map((rid) =>
          wrappedClient.GET('/api/workspaces/{ws}/tasks/{readable_id}', {
            params: { path: { ws, readable_id: rid } },
          }),
        ),
      ),
    );

    if (activeTaskDetailsRequest !== request) return;

    activeTaskDetailsRequest = null;
    detailsLoading.value = false;
    if ('cause' in result) {
      error.value = errorHint(result.cause, 'Failed to load task details');
      return;
    }

    const failedResponse = result.value.find(
      (response) => response.error !== undefined || response.data === undefined,
    );
    if (failedResponse !== undefined) {
      taskDetails.value = new Map();
      error.value = errorHint(failedResponse.error, 'Failed to load task details');
      return;
    }

    const next = new Map<string, TaskDto>();
    result.value.forEach((res, i) => {
      const rid = ids[i];
      if (rid !== undefined && res.data !== undefined) next.set(rid, res.data);
    });

    taskDetails.value = next;
  }

  /**
   * Reconcile the task store after a successful move.
   * Removes the task from its old column, updates its fields (column_id, priority, etc.),
   * and appends it to the new column. The kanban composable may reorder after this.
   */
  function reconcileTask(moved: MovedTaskSummary): void {
    const newColumnId = moved.column_id;

    // A move does not change the task's children; carry the prior count over so
    // the list view's expand affordance survives the optimistic reconcile.
    let priorSubtaskCount = 0;
    for (const [colId, colTasks] of tasks.value) {
      const existing = colTasks.find((t) => t.id === moved.id);
      if (existing === undefined) {
        continue;
      }

      priorSubtaskCount = existing.subtask_count;
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
      board_id: board.value?.id ?? '',
      column_id: newColumnId,
      board_name: board.value?.name ?? '',
      column_name: columns.value.find((c) => c.id === newColumnId)?.name ?? '',
      title: moved.title,
      priority: moved.priority ?? null,
      subtask_count: priorSubtaskCount,
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
      // The custom-field value map (keyed by property-definition key); replaces
      // the task's stored properties wholesale, so callers send the merged map.
      properties?: Record<string, unknown> | null;
    },
  ): Promise<boolean> {
    const { data, error: apiError } = await wrappedClient.PATCH('/api/workspaces/{ws}/tasks/{readable_id}', {
      params: { path: { ws, readable_id: readableId } },
      body: patch,
    });

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to update task');
      return false;
    }

    const summaryPatch: Partial<TaskSummaryDto> & { id: string } = { id: data.id };
    if (patch.title !== undefined) summaryPatch.title = data.title;
    if (patch.priority !== undefined) summaryPatch.priority = data.priority ?? null;
    if (patch.labels !== undefined) summaryPatch.labels = data.labels ?? [];
    updateTaskFields(summaryPatch);

    return true;
  }

  async function deleteTask(ws: string, readableId: string): Promise<boolean> {
    const target = findTaskByReadableId(readableId);

    const { error: apiError } = await wrappedClient.DELETE('/api/workspaces/{ws}/tasks/{readable_id}', {
      params: { path: { ws, readable_id: readableId } },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to delete task');
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
      '/api/workspaces/{ws}/tasks/{readable_id}/assignees',
      {
        params: { path: { ws, readable_id: readableId } },
        body: { assignee_type: principalType, assignee_id: principalId },
      },
    );

    if (apiError !== undefined) {
      const problem = apiError as { hint?: string; detail?: string; status?: number } | undefined;
      if (problem?.status === 409) {
        error.value = problem.hint ?? 'Already assigned to this task';
      } else if (problem?.status === 422) {
        error.value = problem.detail ?? problem.hint ?? 'Cannot assign this user';
      } else {
        error.value = problem?.hint ?? 'Failed to assign task';
      }
      return false;
    }

    // Refresh the board's tasks so the new assignee's avatar shows immediately.
    // Scoped to the active board; cross-board list views update on their next load.
    if (board.value !== null) {
      await loadTasks(ws, board.value.id);
    }

    return true;
  }

  /** Removes a workspace member (user) or agent (api_key) from a task's assignees. */
  async function unassignTask(
    ws: string,
    readableId: string,
    principalType: string,
    principalId: string,
  ): Promise<boolean> {
    const { error: apiError } = await wrappedClient.DELETE(
      '/api/workspaces/{ws}/tasks/{readable_id}/assignees/{assignee_ref}',
      {
        params: {
          path: { ws, readable_id: readableId, assignee_ref: `${principalType}:${principalId}` },
        },
      },
    );

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to unassign task');
      return false;
    }

    if (board.value !== null) {
      await loadTasks(ws, board.value.id);
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
      '/api/workspaces/{ws}/tasks/{readable_id}',
      { params: { path: { ws, readable_id: readableId } } },
    );

    if (getErr !== undefined || source === undefined) {
      error.value = errorHint(getErr, 'Failed to read task');
      return null;
    }

    const { data: created, error: createErr } = await wrappedClient.POST(
      '/api/workspaces/{ws}/boards/{board_id}/tasks',
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
      error.value = errorHint(createErr, 'Failed to duplicate task');
      return null;
    }

    if (source.priority !== undefined && source.priority !== null) {
      await wrappedClient.PATCH('/api/workspaces/{ws}/tasks/{readable_id}', {
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
    const { error: apiError } = await wrappedClient.POST('/api/workspaces/{ws}/tasks/{readable_id}/move', {
      params: { path: { ws, readable_id: readableId } },
      body: { column_id: columnId, before: null, after: null },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to move task');
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
      '/api/workspaces/{ws}/boards/{board_id}/columns',
      { params: { path: { ws, board_id: targetBoardId } } },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to read target board');
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
   * Cached per-board columns for cross-board menus (Change status submenu on
   * TaskViewListView rows). Uses a separate Map so it never overwrites the active
   * `columns` ref that the kanban depends on. Results are cached by boardId and
   * never re-fetched until the store resets.
   */
  const columnsByBoard = new Map<string, ColumnDto[]>();

  async function fetchColumnsForBoard(ws: string, boardId: string): Promise<ColumnDto[]> {
    const cached = columnsByBoard.get(boardId);
    if (cached !== undefined) return cached;

    const { data, error: apiError } = await wrappedClient.GET(
      '/api/workspaces/{ws}/boards/{board_id}/columns',
      { params: { path: { ws, board_id: boardId } } },
    );

    if (apiError !== undefined || data === undefined) {
      return [];
    }

    const sorted = [...data].sort((a, b) => a.position_key.localeCompare(b.position_key));
    columnsByBoard.set(boardId, sorted);
    return sorted;
  }

  /**
   * Fetches a task's direct sub-tasks as row summaries, so the list view can
   * expand a task in place. Returns an empty list on error (the caller shows a
   * collapsed, empty branch rather than surfacing a blocking error); the list is
   * not cached here — the view holds the expansion cache.
   */
  async function loadSubtasks(ws: string, readableId: string): Promise<TaskSummaryDto[]> {
    const { data, error: apiError } = await wrappedClient.GET(
      '/api/workspaces/{ws}/tasks/{readable_id}/subtasks',
      { params: { path: { ws, readable_id: readableId } } },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to load sub-tasks');
      return [];
    }

    return data;
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

  /**
   * Clears all board-scoped state. Called when the active workspace changes so a
   * board (and especially a stale `loadError`) from the previous workspace never
   * bleeds into the next one: a board id is only valid within its own workspace,
   * so loading it against a different workspace 404s, and that error must not
   * survive the switch.
   */
  function reset(): void {
    cancelBoardLoad();
    board.value = null;
    boardSummaries.value = [];
    boardsByProject.value = new Map();
    columns.value = [];
    tasks.value = new Map();
    taskDetails.value = new Map();
    loading.value = false;
    detailsLoading.value = false;
    loadError.value = null;
    error.value = null;
  }

  return {
    board,
    boardSummaries,
    columns,
    loading,
    error,
    loadError,
    reset,
    taskDetails,
    detailsLoading,
    taskDetail,
    loadTaskDetails,
    tasksByColumn,
    filteredTasksByColumn,
    loadBoards,
    loadBoardsForProject,
    boardsFor,
    createBoard,
    renameBoard,
    removeBoard,
    createTask,
    loadBoard,
    loadColumns,
    createColumn,
    updateColumn,
    moveColumn,
    deleteColumn,
    loadTasks,
    loadBoardContents,
    cancelBoardLoad,
    loadSubtasks,
    upsertTaskById,
    reconcileTask,
    applyOptimisticMove,
    snapshotTasks,
    restoreSnapshot,
    findTaskByReadableId,
    updateTaskFields,
    removeTaskById,
    updateTask,
    deleteTask,
    assignTask,
    unassignTask,
    duplicateTask,
    moveTaskToColumn,
    moveTaskToBoard,
    fetchColumnsForBoard,
    _setColumnTasks,
    _setTasksForTest,
  };
});
