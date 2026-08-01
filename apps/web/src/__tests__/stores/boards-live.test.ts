import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET } = vi.hoisted(() => ({ GET: vi.fn() }));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET },
}));

import type { BoardDto, TaskSummaryDto } from '@/stores/boards';
import { useBoardsStore } from '@/stores/boards';

const board: BoardDto = {
  id: 'board-1',
  name: 'Sprint',
  workspace_id: 'ws',
  project_id: 'proj',
  created_at: '',
  updated_at: '',
  created_by: { id: 'u1', type: 'user' },
};

const task = (id: string, columnId: string, title = `Task ${id}`): TaskSummaryDto => ({
  id,
  readable_id: `ATL-${id}`,
  board_id: 'board-1',
  column_id: columnId,
  board_name: 'Sprint',
  column_name: 'Todo',
  title,
  priority: null,
  subtask_count: 0,
  updated_at: '2026-01-01T00:00:00Z',
});

function mockBoardTasks(items: TaskSummaryDto[]): void {
  GET.mockResolvedValue({ data: { items, has_more: false, next_cursor: null }, error: undefined });
}

describe('useBoardsStore.upsertTaskById', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('inserts a task that is not yet on the board', async () => {
    const store = useBoardsStore();
    store.board = board;
    mockBoardTasks([task('1', 'c1')]);

    await store.upsertTaskById('ws', '1');

    expect(store.tasksByColumn('c1')).toHaveLength(1);
    expect(store.tasksByColumn('c1')[0]?.id).toBe('1');
  });

  it('updates an existing task in place without duplicating it (idempotent echo)', async () => {
    const store = useBoardsStore();
    store.board = board;
    store._setTasksForTest({ c1: [task('1', 'c1', 'Old title')] });

    mockBoardTasks([task('1', 'c1', 'New title')]);
    await store.upsertTaskById('ws', '1');

    const c1 = store.tasksByColumn('c1');
    expect(c1).toHaveLength(1);
    expect(c1[0]?.title).toBe('New title');
  });

  it('moves the card between columns when the fetched task changed column', async () => {
    const store = useBoardsStore();
    store.board = board;
    store._setTasksForTest({ c1: [task('1', 'c1')], c2: [] });

    mockBoardTasks([task('1', 'c2')]);
    await store.upsertTaskById('ws', '1');

    expect(store.tasksByColumn('c1')).toHaveLength(0);
    expect(store.tasksByColumn('c2')).toHaveLength(1);
    expect(store.tasksByColumn('c2')[0]?.id).toBe('1');
  });

  it('removes a stale card when the task is no longer on the board', async () => {
    const store = useBoardsStore();
    store.board = board;
    store._setTasksForTest({ c1: [task('1', 'c1'), task('2', 'c1')] });

    mockBoardTasks([task('2', 'c1')]);
    await store.upsertTaskById('ws', '1');

    const c1 = store.tasksByColumn('c1');
    expect(c1).toHaveLength(1);
    expect(c1[0]?.id).toBe('2');
  });

  it('is a no-op when no board is loaded', async () => {
    const store = useBoardsStore();

    await store.upsertTaskById('ws', '1');

    expect(GET).not.toHaveBeenCalled();
  });
});

function mockSubtasks(items: TaskSummaryDto[]): void {
  GET.mockResolvedValue({ data: items, error: undefined });
}

describe('useBoardsStore sub-task branch cache', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('fetches a branch once and serves the cached children afterwards', async () => {
    const store = useBoardsStore();
    mockSubtasks([task('2', 'c1')]);

    await store.expandSubtasks('ws', 'ATL-1');
    await store.expandSubtasks('ws', 'ATL-1');

    expect(GET).toHaveBeenCalledTimes(1);
    expect(store.cachedSubtasks('ATL-1')).toHaveLength(1);
  });

  it('refreshes the branch holding a task that changed elsewhere', async () => {
    const store = useBoardsStore();
    mockSubtasks([task('2', 'c1')]);
    await store.expandSubtasks('ws', 'ATL-1');

    mockSubtasks([{ ...task('2', 'c2'), column_name: 'Done' }]);
    const handled = await store.refreshCachedSubtasksForTask('ws', '2');

    expect(handled).toBe(true);
    expect(store.cachedSubtasks('ATL-1')[0]?.column_id).toBe('c2');
    expect(store.cachedSubtasks('ATL-1')[0]?.column_name).toBe('Done');
  });

  it('reports an unhandled task when no cached branch holds it', async () => {
    const store = useBoardsStore();
    mockSubtasks([task('2', 'c1')]);
    await store.expandSubtasks('ws', 'ATL-1');
    vi.clearAllMocks();

    const handled = await store.refreshCachedSubtasksForTask('ws', '99');

    expect(handled).toBe(false);
    expect(GET).not.toHaveBeenCalled();
  });

  it('drops a deleted task from its cached branch', async () => {
    const store = useBoardsStore();
    mockSubtasks([task('2', 'c1'), task('3', 'c1')]);
    await store.expandSubtasks('ws', 'ATL-1');

    store.removeCachedSubtask('2');

    expect(store.cachedSubtasks('ATL-1').map((child) => child.id)).toEqual(['3']);
  });

  it('never refetches a branch that was never expanded', async () => {
    const store = useBoardsStore();

    await store.refreshAllCachedSubtasks('ws');

    expect(GET).not.toHaveBeenCalled();
  });
});
