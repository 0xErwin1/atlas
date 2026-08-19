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
