import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { POST } = vi.hoisted(() => ({ POST: vi.fn() }));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { POST },
}));

import { useKanbanMove } from '@/composables/useKanbanMove';
import { type TaskSummaryDto, useBoardsStore } from '@/stores/boards';

const task = (id: string, readableId: string, columnId: string): TaskSummaryDto => ({
  id,
  readable_id: readableId,
  column_id: columnId,
  board_name: 'Board',
  column_name: 'Todo',
  title: `Task ${id}`,
  priority: null,
  updated_at: '2026-01-01T00:00:00Z',
});

const taskDto = (id: string, readableId: string, columnId: string) => ({
  id,
  readable_id: readableId,
  column_id: columnId,
  board_id: 'board-1',
  title: `Task ${id}`,
  description: '',
  priority: 'high',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-02T00:00:00Z',
  workspace_id: 'ws',
  project_id: 'proj',
  created_by: { id: 'u1', name: 'User', principal_type: 'user' },
  labels: [],
});

describe('useKanbanMove', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('happy path: optimistic move then 200 — store reconciled with returned TaskDto (REQ-W21)', async () => {
    const store = useBoardsStore();
    store._setTasksForTest({
      c1: [task('t1', 'ATL-1', 'c1'), task('t2', 'ATL-2', 'c1')],
      c2: [task('t3', 'ATL-3', 'c2')],
    });

    const returnedTask = taskDto('t1', 'ATL-1', 'c2');
    POST.mockResolvedValueOnce({ data: returnedTask, error: undefined });

    const { move } = useKanbanMove('ws');
    const result = await move('ATL-1', 'c2', 0);

    expect(result.ok).toBe(true);

    const c1 = store.tasksByColumn('c1');
    const c2 = store.tasksByColumn('c2');

    expect(c1).toHaveLength(1);
    expect(c1[0]?.id).toBe('t2');

    expect(c2.some((t) => t.id === 't1')).toBe(true);
    const moved = c2.find((t) => t.id === 't1');
    expect(moved?.priority).toBe('high');
  });

  it('non-409 error: store is EXACTLY restored to pre-move snapshot (REQ-W21)', async () => {
    const store = useBoardsStore();
    store._setTasksForTest({
      c1: [task('t1', 'ATL-1', 'c1'), task('t2', 'ATL-2', 'c1')],
      c2: [task('t3', 'ATL-3', 'c2')],
    });

    POST.mockResolvedValueOnce({
      data: undefined,
      error: { type: 'urn:atlas:error:forbidden', hint: 'No permission', status: 403 },
    });

    const { move } = useKanbanMove('ws');
    const result = await move('ATL-1', 'c2', 0);

    expect(result.ok).toBe(false);
    expect(result.hint).toBe('No permission');

    const c1 = store.tasksByColumn('c1');
    const c2 = store.tasksByColumn('c2');

    expect(c1).toHaveLength(2);
    expect(c1[0]?.id).toBe('t1');
    expect(c1[1]?.id).toBe('t2');
    expect(c2).toHaveLength(1);
    expect(c2[0]?.id).toBe('t3');
  });

  it('409 once then success on retry — ends consistent, no rollback (REQ-W21)', async () => {
    const store = useBoardsStore();
    store._setTasksForTest({
      c1: [task('t1', 'ATL-1', 'c1'), task('t2', 'ATL-2', 'c1')],
      c2: [],
    });

    const returnedTask = taskDto('t1', 'ATL-1', 'c2');

    POST.mockResolvedValueOnce({
      data: undefined,
      error: {
        type: 'urn:atlas:error:position-exhausted',
        hint: 'Retry the move; the server attempted to rebalance column positions.',
        status: 409,
      },
    }).mockResolvedValueOnce({ data: returnedTask, error: undefined });

    const { move } = useKanbanMove('ws');
    const result = await move('ATL-1', 'c2', 0);

    expect(result.ok).toBe(true);

    expect(POST).toHaveBeenCalledTimes(2);

    const c1 = store.tasksByColumn('c1');
    const c2 = store.tasksByColumn('c2');
    expect(c1).toHaveLength(1);
    expect(c2.some((t) => t.id === 't1')).toBe(true);
  });

  it('409 twice (persistent): rolled back to snapshot + hint surfaced (REQ-W21)', async () => {
    const store = useBoardsStore();
    store._setTasksForTest({
      c1: [task('t1', 'ATL-1', 'c1'), task('t2', 'ATL-2', 'c1')],
      c2: [task('t3', 'ATL-3', 'c2')],
    });

    const posExhaustedError = {
      type: 'urn:atlas:error:position-exhausted',
      hint: 'Retry the move; the server attempted to rebalance column positions.',
      status: 409,
    };

    POST.mockResolvedValue({ data: undefined, error: posExhaustedError });

    const { move } = useKanbanMove('ws');
    const result = await move('ATL-1', 'c2', 0);

    expect(result.ok).toBe(false);
    expect(result.hint).toContain('rebalance');

    expect(POST).toHaveBeenCalledTimes(2);

    const c1 = store.tasksByColumn('c1');
    const c2 = store.tasksByColumn('c2');

    expect(c1).toHaveLength(2);
    expect(c1[0]?.id).toBe('t1');
    expect(c1[1]?.id).toBe('t2');
    expect(c2).toHaveLength(1);
    expect(c2[0]?.id).toBe('t3');
  });

  it('move within the same column: correct neighbors computed, task moves to target index', async () => {
    const store = useBoardsStore();
    store._setTasksForTest({
      c1: [task('t1', 'ATL-1', 'c1'), task('t2', 'ATL-2', 'c1'), task('t3', 'ATL-3', 'c1')],
    });

    const returnedTask = taskDto('t1', 'ATL-1', 'c1');
    POST.mockResolvedValueOnce({ data: returnedTask, error: undefined });

    const { move } = useKanbanMove('ws');
    await move('ATL-1', 'c1', 2);

    const [callArgs] = POST.mock.calls;
    const body = (callArgs as unknown[])[1] as {
      body: { column_id: string; before?: string | null; after?: string | null };
    };

    expect(body.body.column_id).toBe('c1');

    const c1 = store.tasksByColumn('c1');
    expect(c1).toHaveLength(3);
    expect(c1[c1.length - 1]?.id).toBe('t1');
  });

  it('move across columns: POST is called with correct column_id (REQ-W21)', async () => {
    const store = useBoardsStore();
    store._setTasksForTest({
      c1: [task('t1', 'ATL-1', 'c1'), task('t2', 'ATL-2', 'c1')],
      c2: [task('t3', 'ATL-3', 'c2'), task('t4', 'ATL-4', 'c2')],
    });

    const returnedTask = taskDto('t1', 'ATL-1', 'c2');
    POST.mockResolvedValueOnce({ data: returnedTask, error: undefined });

    const { move } = useKanbanMove('ws');
    await move('ATL-1', 'c2', 1);

    const [callArgs] = POST.mock.calls;
    const body = (callArgs as unknown[])[1] as { body: { column_id: string } };

    expect(body.body.column_id).toBe('c2');
  });

  it('server 200 with different column_id than optimistic: store reflects the TaskDto (not the guess)', async () => {
    const store = useBoardsStore();
    store._setTasksForTest({
      c1: [task('t1', 'ATL-1', 'c1')],
      c2: [],
      c3: [],
    });

    const returnedTask = taskDto('t1', 'ATL-1', 'c3');
    POST.mockResolvedValueOnce({ data: returnedTask, error: undefined });

    const { move } = useKanbanMove('ws');
    const result = await move('ATL-1', 'c2', 0);

    expect(result.ok).toBe(true);

    expect(store.tasksByColumn('c2').some((t) => t.id === 't1')).toBe(false);
    expect(store.tasksByColumn('c3').some((t) => t.id === 't1')).toBe(true);
  });
});
