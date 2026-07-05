import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET } = vi.hoisted(() => ({ GET: vi.fn() }));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET },
}));

import type { ColumnDto, TaskSummaryDto } from '@/stores/boards';
import { useBoardsStore } from '@/stores/boards';

const col = (id: string, positionKey: string): ColumnDto => ({
  id,
  board_id: 'board-1',
  name: `Col ${id}`,
  position_key: positionKey,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
});

const task = (id: string, readableId: string, columnId: string): TaskSummaryDto => ({
  id,
  readable_id: readableId,
  board_id: 'board-1',
  column_id: columnId,
  board_name: 'Board',
  column_name: 'Todo',
  title: `Task ${id}`,
  priority: null,
  subtask_count: 0,
  updated_at: '2026-01-01T00:00:00Z',
});

describe('useBoardsStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('returns a stable array reference for an empty column (guards the kanban render loop)', () => {
    const store = useBoardsStore();
    const first = store.tasksByColumn('empty-col');
    const second = store.tasksByColumn('empty-col');

    expect(first).toEqual([]);
    expect(first).toBe(second);
  });

  it('loadBoard fetches the board and stores it (REQ-W20)', async () => {
    GET.mockResolvedValueOnce({
      data: {
        id: 'board-1',
        name: 'Sprint 1',
        workspace_id: 'ws',
        project_id: 'proj',
        created_at: '',
        updated_at: '',
        created_by: { id: 'u1', name: 'User', principal_type: 'user' },
      },
      error: undefined,
    });

    const store = useBoardsStore();
    await store.loadBoard('ws', 'board-1');

    expect(store.board?.id).toBe('board-1');
    expect(store.board?.name).toBe('Sprint 1');
    expect(store.error).toBeNull();
  });

  it('loadColumns stores columns sorted by position_key (REQ-W20)', async () => {
    GET.mockResolvedValue({
      data: [col('c2', 'n'), col('c1', 'a'), col('c3', 'z')],
      error: undefined,
    });

    const store = useBoardsStore();
    await store.loadColumns('ws', 'board-1');

    expect(store.columns).toHaveLength(3);
    expect(store.columns[0]?.id).toBe('c1');
    expect(store.columns[1]?.id).toBe('c2');
    expect(store.columns[2]?.id).toBe('c3');
  });

  it('loadTasks groups tasks by column and preserves server order (REQ-W20)', async () => {
    GET.mockResolvedValue({
      data: {
        items: [task('t1', 'ATL-1', 'c1'), task('t2', 'ATL-2', 'c1'), task('t3', 'ATL-3', 'c2')],
        has_more: false,
        next_cursor: null,
      },
      error: undefined,
    });

    const store = useBoardsStore();
    await store.loadTasks('ws', 'board-1');

    const c1Tasks = store.tasksByColumn('c1');
    const c2Tasks = store.tasksByColumn('c2');

    expect(c1Tasks).toHaveLength(2);
    expect(c1Tasks[0]?.id).toBe('t1');
    expect(c1Tasks[1]?.id).toBe('t2');
    expect(c2Tasks).toHaveLength(1);
    expect(c2Tasks[0]?.id).toBe('t3');
  });

  it('loadTasks surfaces hint on error', async () => {
    GET.mockResolvedValue({ data: undefined, error: { hint: 'board not found' } });

    const store = useBoardsStore();
    await store.loadTasks('ws', 'board-1');

    expect(store.loadError).toBe('board not found');
  });

  it('reset clears a stale load error and board-scoped state (cross-workspace bleed)', async () => {
    GET.mockResolvedValue({ data: undefined, error: { hint: 'board not found' } });

    const store = useBoardsStore();
    await store.loadTasks('ws', 'board-1');
    store._setTasksForTest({ c1: [task('t1', 'ATL-1', 'c1')] });

    expect(store.loadError).toBe('board not found');

    store.reset();

    expect(store.loadError).toBeNull();
    expect(store.board).toBeNull();
    expect(store.columns).toHaveLength(0);
    expect(store.tasksByColumn('c1')).toHaveLength(0);
  });

  it('reconcileTask moves a task to a new column and updates its id (REQ-W21)', () => {
    const store = useBoardsStore();

    store._setTasksForTest({
      c1: [task('t1', 'ATL-1', 'c1'), task('t2', 'ATL-2', 'c1')],
      c2: [task('t3', 'ATL-3', 'c2')],
    });

    store.reconcileTask({
      id: 't1',
      readable_id: 'ATL-1',
      column_id: 'c2',
      title: 'Task t1',
      priority: 'high',
      updated_at: '2026-01-02T00:00:00Z',
    });

    expect(store.tasksByColumn('c1')).toHaveLength(1);
    expect(store.tasksByColumn('c1')[0]?.id).toBe('t2');
    expect(store.tasksByColumn('c2')).toHaveLength(2);
    const movedTask = store.tasksByColumn('c2').find((t) => t.id === 't1');
    expect(movedTask).toBeDefined();
    expect(movedTask?.priority).toBe('high');
  });

  it('reconcileTask handles a task moving to an empty column', () => {
    const store = useBoardsStore();

    store._setTasksForTest({
      c1: [task('t1', 'ATL-1', 'c1')],
      c2: [],
    });

    store.reconcileTask({
      id: 't1',
      readable_id: 'ATL-1',
      column_id: 'c2',
      title: 'Task t1',
      priority: null,
      updated_at: '2026-01-02T00:00:00Z',
    });

    expect(store.tasksByColumn('c1')).toHaveLength(0);
    expect(store.tasksByColumn('c2')).toHaveLength(1);
  });

  it('applyOptimisticMove moves task to target column at the given index', () => {
    const store = useBoardsStore();

    store._setTasksForTest({
      c1: [task('t1', 'ATL-1', 'c1'), task('t2', 'ATL-2', 'c1')],
      c2: [task('t3', 'ATL-3', 'c2')],
    });

    store.applyOptimisticMove('t1', 'c2', 0);

    const c1 = store.tasksByColumn('c1');
    const c2 = store.tasksByColumn('c2');

    expect(c1).toHaveLength(1);
    expect(c1[0]?.id).toBe('t2');
    expect(c2).toHaveLength(2);
    expect(c2[0]?.id).toBe('t1');
    expect(c2[1]?.id).toBe('t3');
  });

  it('snapshotTasks returns a deep copy that does not reflect subsequent mutations', () => {
    const store = useBoardsStore();

    store._setTasksForTest({
      c1: [task('t1', 'ATL-1', 'c1')],
    });

    const snapshot = store.snapshotTasks();
    store.applyOptimisticMove('t1', 'c2', 0);

    const snapshotC1 = snapshot.get('c1');
    expect(snapshotC1).toHaveLength(1);
    expect(snapshotC1?.[0]?.id).toBe('t1');
  });

  it('restoreSnapshot reverts to the captured state exactly', () => {
    const store = useBoardsStore();

    store._setTasksForTest({
      c1: [task('t1', 'ATL-1', 'c1'), task('t2', 'ATL-2', 'c1')],
      c2: [task('t3', 'ATL-3', 'c2')],
    });

    const snapshot = store.snapshotTasks();
    store.applyOptimisticMove('t1', 'c2', 0);

    expect(store.tasksByColumn('c1')).toHaveLength(1);

    store.restoreSnapshot(snapshot);

    expect(store.tasksByColumn('c1')).toHaveLength(2);
    expect(store.tasksByColumn('c1')[0]?.id).toBe('t1');
    expect(store.tasksByColumn('c1')[1]?.id).toBe('t2');
    expect(store.tasksByColumn('c2')).toHaveLength(1);
    expect(store.tasksByColumn('c2')[0]?.id).toBe('t3');
  });
});
