import { wrappedClient } from '@/api/wrapper';
import { type TaskSummaryDto, useBoardsStore } from '@/stores/boards';

const POSITION_EXHAUSTED_TYPE = 'urn:atlas:error:position-exhausted';

export interface MoveResult {
  ok: boolean;
  hint?: string;
}

interface ApiProblem {
  type?: string;
  hint?: string;
  status?: number;
}

interface MovedTaskData {
  id: string;
  readable_id: string;
  column_id: string;
  title: string;
  priority?: string | null;
  updated_at: string;
}

function isPositionExhausted(err: unknown): err is ApiProblem {
  if (typeof err !== 'object' || err === null) {
    return false;
  }

  const e = err as ApiProblem;
  return e.type === POSITION_EXHAUSTED_TYPE;
}

/**
 * Compute the before/after neighbor IDs for a POST move request.
 *
 * `after` = the task immediately before the target slot (the one the moving task will follow).
 * `before` = the task immediately after the target slot (the one that will follow the moving task).
 * Null means the slot is at the boundary on that side.
 *
 * `destTasks` must be the destination column tasks EXCLUDING the moving task,
 * so that indexes are correct when the task moves within the same column.
 */
function computeNeighbors(
  destTasks: TaskSummaryDto[],
  toIndex: number,
): { before: string | null; after: string | null } {
  const afterTask = toIndex > 0 ? destTasks[toIndex - 1] : undefined;
  const beforeTask = destTasks[toIndex];

  return {
    after: afterTask?.id ?? null,
    before: beforeTask?.id ?? null,
  };
}

/**
 * useKanbanMove — composable for optimistic task moves with rollback (REQ-W21).
 *
 * move(readableId, columnId, toIndex):
 *   1. Snapshot the board store's task ordering.
 *   2. Apply the move optimistically (instant UI update).
 *   3. POST /v1/workspaces/{ws}/tasks/{readable_id}/move.
 *   4. On 200 → reconcile the store with the returned TaskDto (canonical column + position).
 *   5. On 409 position-exhausted → retry once. If the retry also fails → rollback + hint.
 *   6. On any other error → rollback + hint.
 *
 * Never leaves the store in a torn or inconsistent state.
 */
export function useKanbanMove(ws: string) {
  const store = useBoardsStore();

  async function attemptMove(
    readableId: string,
    columnId: string,
    toIndex: number,
  ): Promise<{ data: unknown; error: unknown | undefined }> {
    const destTasks = store.tasksByColumn(columnId).filter((t) => t.readable_id !== readableId);
    const neighbors = computeNeighbors(destTasks, toIndex);

    const result = await wrappedClient.POST('/v1/workspaces/{ws}/tasks/{readable_id}/move', {
      params: { path: { ws, readable_id: readableId } },
      body: {
        column_id: columnId,
        before: neighbors.before,
        after: neighbors.after,
      },
    });

    return { data: result.data, error: result.error };
  }

  function applyServerResult(
    snapshot: Map<string, TaskSummaryDto[]>,
    taskId: string,
    moved: MovedTaskData,
    toIndex: number,
  ): void {
    store.restoreSnapshot(snapshot);
    store.applyOptimisticMove(taskId, moved.column_id, toIndex);

    store.updateTaskFields({
      id: moved.id,
      readable_id: moved.readable_id,
      column_id: moved.column_id,
      title: moved.title,
      priority: moved.priority ?? null,
      updated_at: moved.updated_at,
    });
  }

  async function move(readableId: string, columnId: string, toIndex: number): Promise<MoveResult> {
    const task = store.findTaskByReadableId(readableId);
    const snapshot = store.snapshotTasks();

    if (task !== undefined) {
      store.applyOptimisticMove(task.id, columnId, toIndex);
    }

    const firstAttempt = await attemptMove(readableId, columnId, toIndex);

    if (firstAttempt.error === undefined && firstAttempt.data !== undefined) {
      const moved = firstAttempt.data as MovedTaskData;
      applyServerResult(snapshot, task?.id ?? moved.id, moved, toIndex);
      return { ok: true };
    }

    if (!isPositionExhausted(firstAttempt.error)) {
      store.restoreSnapshot(snapshot);
      const hint = (firstAttempt.error as ApiProblem | undefined)?.hint ?? 'Move failed';
      return { ok: false, hint };
    }

    const retryAttempt = await attemptMove(readableId, columnId, toIndex);

    if (retryAttempt.error === undefined && retryAttempt.data !== undefined) {
      const moved = retryAttempt.data as MovedTaskData;
      applyServerResult(snapshot, task?.id ?? moved.id, moved, toIndex);
      return { ok: true };
    }

    store.restoreSnapshot(snapshot);
    const hint = (retryAttempt.error as ApiProblem | undefined)?.hint ?? 'Move failed after retry';
    return { ok: false, hint };
  }

  return { move };
}
