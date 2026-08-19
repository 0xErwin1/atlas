import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET } = vi.hoisted(() => ({ GET: vi.fn() }));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET },
}));

import type { BoardDto, ColumnDto, TaskSummaryDto } from '@/stores/boards';
import { useBoardsStore } from '@/stores/boards';

const board: BoardDto = {
  id: 'board-1',
  name: 'Sprint',
  workspace_id: 'ws',
  project_id: 'proj',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
  created_by: { id: 'u1', type: 'user' },
};

const col = (id: string, positionKey: string): ColumnDto => ({
  id,
  board_id: 'board-1',
  name: `Col ${id}`,
  position_key: positionKey,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
});

const task = (id: string, columnId: string, overrides: Partial<TaskSummaryDto> = {}): TaskSummaryDto => ({
  id,
  readable_id: `ATL-${id}`,
  board_id: 'board-1',
  column_id: columnId,
  board_name: 'Sprint',
  column_name: 'Todo',
  title: `Task ${id}`,
  priority: null,
  subtask_count: 0,
  updated_at: '2026-01-01T00:00:00Z',
  ...overrides,
});

function mockBoard(columns: ColumnDto[], items: TaskSummaryDto[]): void {
  GET.mockImplementation((path: string) => {
    if (path.endsWith('/columns')) return Promise.resolve({ data: columns, error: undefined });
    if (path.endsWith('/tasks')) {
      return Promise.resolve({ data: { items, has_more: false, next_cursor: null }, error: undefined });
    }
    return Promise.resolve({ data: board, error: undefined });
  });
}

/**
 * A task whose `readable_id` counts every read, so a lookup that scans the board
 * is distinguishable from one that goes through an index.
 */
function countingTask(id: string, columnId: string, counter: { reads: number }): TaskSummaryDto {
  const base = task(id, columnId);
  const readableId = base.readable_id;

  return Object.defineProperty({ ...base }, 'readable_id', {
    enumerable: true,
    configurable: true,
    get() {
      counter.reads += 1;
      return readableId;
    },
  }) as TaskSummaryDto;
}

describe('boards store — findTaskByReadableId', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('does not scan the board on every lookup', () => {
    const store = useBoardsStore();
    const counter = { reads: 0 };
    const boardTasks = Array.from({ length: 500 }, (_, i) => countingTask(`t${i}`, 'c1', counter));

    store._setColumnTasks('c1', boardTasks);
    expect(store.findTaskByReadableId('ATL-t499')?.id).toBe('t499');

    counter.reads = 0;
    for (let i = 0; i < 50; i += 1) {
      expect(store.findTaskByReadableId('ATL-t499')?.id).toBe('t499');
    }

    expect(counter.reads).toBe(0);
  });

  it('returns undefined for a task the board does not hold', () => {
    const store = useBoardsStore();
    store._setTasksForTest({ c1: [task('t1', 'c1')] });

    expect(store.findTaskByReadableId('ATL-nope')).toBeUndefined();
  });

  it('resolves the task the store currently holds after a move', () => {
    const store = useBoardsStore();
    store.board = board;
    store.columns = [col('c1', 'a'), col('c2', 'b')];
    store._setTasksForTest({ c1: [task('t1', 'c1')], c2: [] });

    expect(store.findTaskByReadableId('ATL-t1')?.column_id).toBe('c1');

    store.reconcileTask({
      id: 't1',
      readable_id: 'ATL-t1',
      column_id: 'c2',
      title: 'Task t1',
      priority: null,
      updated_at: '2026-01-02T00:00:00Z',
    });

    expect(store.findTaskByReadableId('ATL-t1')?.column_id).toBe('c2');
    expect(store.findTaskByReadableId('ATL-t1')).toBe(store.tasksByColumn('c2')[0]);
  });

  it('resolves the task the store currently holds after an optimistic move', () => {
    const store = useBoardsStore();
    store._setTasksForTest({ c1: [task('t1', 'c1'), task('t2', 'c1')], c2: [] });

    store.applyOptimisticMove('t1', 'c2', 0);

    expect(store.findTaskByReadableId('ATL-t1')?.column_id).toBe('c2');
    expect(store.findTaskByReadableId('ATL-t1')).toBe(store.tasksByColumn('c2')[0]);
    expect(store.findTaskByReadableId('ATL-t2')).toBe(store.tasksByColumn('c1')[0]);
  });

  it('resolves the updated task after a field update', () => {
    const store = useBoardsStore();
    store._setTasksForTest({ c1: [task('t1', 'c1')] });

    store.updateTaskFields({ id: 't1', title: 'Renamed' });

    expect(store.findTaskByReadableId('ATL-t1')?.title).toBe('Renamed');
    expect(store.findTaskByReadableId('ATL-t1')).toBe(store.tasksByColumn('c1')[0]);
  });

  it('resolves tasks added, changed and dropped by a republish', async () => {
    const store = useBoardsStore();
    mockBoard([col('c1', 'a'), col('c2', 'b')], [task('t1', 'c1'), task('t2', 'c2')]);

    await store.loadBoardContents('ws', 'board-1');

    expect(store.findTaskByReadableId('ATL-t1')?.column_id).toBe('c1');
    expect(store.findTaskByReadableId('ATL-t3')).toBeUndefined();

    mockBoard([col('c1', 'a'), col('c2', 'b')], [task('t1', 'c2', { title: 'Renamed' }), task('t3', 'c1')]);
    await store.loadBoardContents('ws', 'board-1', undefined, { background: true });

    expect(store.findTaskByReadableId('ATL-t1')?.title).toBe('Renamed');
    expect(store.findTaskByReadableId('ATL-t1')?.column_id).toBe('c2');
    expect(store.findTaskByReadableId('ATL-t3')?.column_id).toBe('c1');
    expect(store.findTaskByReadableId('ATL-t2')).toBeUndefined();
  });

  it('drops a task removed from the board', () => {
    const store = useBoardsStore();
    store._setTasksForTest({ c1: [task('t1', 'c1'), task('t2', 'c1')] });

    store.removeTaskById('t1');

    expect(store.findTaskByReadableId('ATL-t1')).toBeUndefined();
    expect(store.findTaskByReadableId('ATL-t2')?.id).toBe('t2');
  });

  it('follows a rollback to a snapshot', () => {
    const store = useBoardsStore();
    store._setTasksForTest({ c1: [task('t1', 'c1')], c2: [] });
    const snapshot = store.snapshotTasks();

    store.applyOptimisticMove('t1', 'c2', 0);
    expect(store.findTaskByReadableId('ATL-t1')?.column_id).toBe('c2');

    store.restoreSnapshot(snapshot);
    expect(store.findTaskByReadableId('ATL-t1')?.column_id).toBe('c1');
  });
});
