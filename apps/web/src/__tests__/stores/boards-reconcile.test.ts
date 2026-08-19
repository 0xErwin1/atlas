import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET } = vi.hoisted(() => ({ GET: vi.fn() }));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET },
}));

import type { BoardDto, ColumnDto, TaskSummaryDto } from '@/stores/boards';
import { useBoardsStore } from '@/stores/boards';
import { useUiStore } from '@/stores/ui';

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

/** Answers the three board GETs (board, columns, tasks) from one fixture set. */
function mockBoard(columns: ColumnDto[], items: TaskSummaryDto[]): void {
  GET.mockImplementation((path: string) => {
    if (path.endsWith('/columns')) return Promise.resolve({ data: columns, error: undefined });
    if (path.endsWith('/tasks')) {
      return Promise.resolve({ data: { items, has_more: false, next_cursor: null }, error: undefined });
    }
    return Promise.resolve({ data: board, error: undefined });
  });
}

describe('boards store — reconcileTask', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('keeps labels, assignees and estimate that the move did not change', () => {
    const store = useBoardsStore();
    store.board = board;
    store.columns = [col('c1', 'a'), col('c2', 'b')];
    store._setTasksForTest({
      c1: [
        task('t1', 'c1', {
          labels: ['bug', 'urgent'],
          assignees: [{ id: 'u1', type: 'user', display_name: 'Ada' }],
          estimate: 3,
          subtask_count: 2,
        }),
      ],
    });

    store.reconcileTask({
      id: 't1',
      readable_id: 'ATL-t1',
      column_id: 'c2',
      title: 'Task t1',
      priority: 'high',
      updated_at: '2026-01-02T00:00:00Z',
    });

    const moved = store.tasksByColumn('c2')[0];
    expect(moved?.labels).toEqual(['bug', 'urgent']);
    expect(moved?.assignees).toEqual([{ id: 'u1', type: 'user', display_name: 'Ada' }]);
    expect(moved?.estimate).toBe(3);
    expect(moved?.subtask_count).toBe(2);
    expect(moved?.column_id).toBe('c2');
    expect(moved?.priority).toBe('high');
    expect(moved?.updated_at).toBe('2026-01-02T00:00:00Z');
    expect(moved?.column_name).toBe('Col c2');
  });

  it('still lands a task the board does not hold yet', () => {
    const store = useBoardsStore();
    store.board = board;
    store.columns = [col('c1', 'a')];

    store.reconcileTask({
      id: 't9',
      readable_id: 'ATL-t9',
      column_id: 'c1',
      title: 'New',
      priority: null,
      updated_at: '2026-01-02T00:00:00Z',
    });

    expect(store.tasksByColumn('c1')).toHaveLength(1);
    expect(store.tasksByColumn('c1')[0]?.id).toBe('t9');
  });
});

describe('boards store — publish identity', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('keeps every object identity when a background refresh republishes the same data', async () => {
    const store = useBoardsStore();
    mockBoard([col('c1', 'a'), col('c2', 'b')], [task('t1', 'c1'), task('t2', 'c2')]);

    await store.loadBoardContents('ws', 'board-1');

    const firstBoard = store.board;
    const firstColumns = store.columns;
    const firstColumn = store.columns[0];
    const firstList = store.tasksByColumn('c1');
    const firstTask = firstList[0];

    await store.loadBoardContents('ws', 'board-1', undefined, { background: true });

    expect(store.board).toBe(firstBoard);
    expect(store.columns).toBe(firstColumns);
    expect(store.columns[0]).toBe(firstColumn);
    expect(store.tasksByColumn('c1')).toBe(firstList);
    expect(store.tasksByColumn('c1')[0]).toBe(firstTask);
  });

  it('keeps the filtered-list memo valid across a no-op publish', async () => {
    const store = useBoardsStore();
    const ui = useUiStore();
    ui.setTaskFilterText('Task');
    mockBoard([col('c1', 'a')], [task('t1', 'c1'), task('t2', 'c1')]);

    await store.loadBoardContents('ws', 'board-1');
    const firstFiltered = store.filteredTasksByColumn('c1');

    await store.loadBoardContents('ws', 'board-1', undefined, { background: true });

    expect(store.filteredTasksByColumn('c1')).toBe(firstFiltered);
  });

  it('replaces only what changed when one task is updated', async () => {
    const store = useBoardsStore();
    mockBoard([col('c1', 'a'), col('c2', 'b')], [task('t1', 'c1'), task('t2', 'c2')]);

    await store.loadBoardContents('ws', 'board-1');
    const untouchedList = store.tasksByColumn('c2');
    const untouchedTask = untouchedList[0];
    const changingTask = store.tasksByColumn('c1')[0];

    mockBoard([col('c1', 'a'), col('c2', 'b')], [task('t1', 'c1', { title: 'Renamed' }), task('t2', 'c2')]);
    await store.loadBoardContents('ws', 'board-1', undefined, { background: true });

    expect(store.tasksByColumn('c2')).toBe(untouchedList);
    expect(store.tasksByColumn('c2')[0]).toBe(untouchedTask);
    expect(store.tasksByColumn('c1')[0]).not.toBe(changingTask);
    expect(store.tasksByColumn('c1')[0]?.title).toBe('Renamed');
  });

  it('drops a column that no longer has tasks and adds a new one', async () => {
    const store = useBoardsStore();
    mockBoard([col('c1', 'a'), col('c2', 'b')], [task('t1', 'c1')]);
    await store.loadBoardContents('ws', 'board-1');

    mockBoard([col('c1', 'a'), col('c2', 'b')], [task('t1', 'c2')]);
    await store.loadBoardContents('ws', 'board-1', undefined, { background: true });

    expect(store.tasksByColumn('c1')).toEqual([]);
    expect(store.tasksByColumn('c2')).toHaveLength(1);
  });
});

describe('boards store — upsertTaskById identity', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('keeps the existing card objects when the refetch echoes what is already held', async () => {
    const store = useBoardsStore();
    store.board = board;
    mockBoard([col('c1', 'a')], [task('t1', 'c1'), task('t2', 'c1')]);
    await store.loadBoardContents('ws', 'board-1');

    const firstList = store.tasksByColumn('c1');
    const firstTask = firstList[0];

    await store.upsertTaskById('ws', 't1');

    expect(store.tasksByColumn('c1')).toBe(firstList);
    expect(store.tasksByColumn('c1')[0]).toBe(firstTask);
  });

  it('replaces only the task that actually changed', async () => {
    const store = useBoardsStore();
    store.board = board;
    mockBoard([col('c1', 'a')], [task('t1', 'c1'), task('t2', 'c1')]);
    await store.loadBoardContents('ws', 'board-1');

    const untouched = store.tasksByColumn('c1')[1];

    mockBoard([col('c1', 'a')], [task('t1', 'c1', { title: 'Renamed' }), task('t2', 'c1')]);
    await store.upsertTaskById('ws', 't1');

    expect(store.tasksByColumn('c1')[0]?.title).toBe('Renamed');
    expect(store.tasksByColumn('c1')[1]).toBe(untouched);
  });
});
