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
