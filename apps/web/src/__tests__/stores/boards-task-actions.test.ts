import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET, POST, PATCH, DELETE } = vi.hoisted(() => ({
  GET: vi.fn(),
  POST: vi.fn(),
  PATCH: vi.fn(),
  DELETE: vi.fn(),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET, POST, PATCH, DELETE },
}));

import type { TaskSummaryDto } from '@/stores/boards';
import { useBoardsStore } from '@/stores/boards';

const task = (
  id: string,
  readableId: string,
  columnId: string,
  priority: string | null = null,
): TaskSummaryDto => ({
  id,
  readable_id: readableId,
  board_id: 'board-1',
  column_id: columnId,
  board_name: 'Board',
  column_name: 'Todo',
  title: `Task ${id}`,
  priority,
  updated_at: '2026-01-01T00:00:00Z',
});

describe('boards store — task context-menu actions', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('updateTask patches the task and reflects priority in the local summary', async () => {
    const store = useBoardsStore();
    store._setTasksForTest({ 'col-1': [task('t1', 'AB-1', 'col-1', null)] });

    PATCH.mockResolvedValueOnce({
      data: { id: 't1', readable_id: 'AB-1', title: 'Task t1', priority: 'high' },
      error: undefined,
    });

    const ok = await store.updateTask('ws', 'AB-1', { priority: 'high' });

    expect(ok).toBe(true);
    expect(PATCH).toHaveBeenCalledWith('/v1/workspaces/{ws}/tasks/{readable_id}', {
      params: { path: { ws: 'ws', readable_id: 'AB-1' } },
      body: { priority: 'high' },
    });
    expect(store.findTaskByReadableId('AB-1')?.priority).toBe('high');
  });

  it('deleteTask removes the task from its column on success', async () => {
    const store = useBoardsStore();
    store._setTasksForTest({ 'col-1': [task('t1', 'AB-1', 'col-1'), task('t2', 'AB-2', 'col-1')] });

    DELETE.mockResolvedValueOnce({ error: undefined });

    const ok = await store.deleteTask('ws', 'AB-1');

    expect(ok).toBe(true);
    expect(store.findTaskByReadableId('AB-1')).toBeUndefined();
    expect(store.findTaskByReadableId('AB-2')).toBeDefined();
  });

  it('deleteTask keeps the task and sets an error hint on failure', async () => {
    const store = useBoardsStore();
    store._setTasksForTest({ 'col-1': [task('t1', 'AB-1', 'col-1')] });

    DELETE.mockResolvedValueOnce({ error: { hint: 'nope' } });

    const ok = await store.deleteTask('ws', 'AB-1');

    expect(ok).toBe(false);
    expect(store.error).toBe('nope');
    expect(store.findTaskByReadableId('AB-1')).toBeDefined();
  });

  it('moveTaskToColumn posts a move with null neighbours', async () => {
    const store = useBoardsStore();
    POST.mockResolvedValueOnce({ data: { id: 't1' }, error: undefined });

    const ok = await store.moveTaskToColumn('ws', 'AB-1', 'col-2');

    expect(ok).toBe(true);
    expect(POST).toHaveBeenCalledWith('/v1/workspaces/{ws}/tasks/{readable_id}/move', {
      params: { path: { ws: 'ws', readable_id: 'AB-1' } },
      body: { column_id: 'col-2', before: null, after: null },
    });
  });

  it('moveTaskToBoard moves into the target board first column', async () => {
    const store = useBoardsStore();
    GET.mockResolvedValueOnce({
      data: [
        { id: 'c-b', board_id: 'b2', name: 'Doing', position_key: 'n', created_at: '', updated_at: '' },
        { id: 'c-a', board_id: 'b2', name: 'Todo', position_key: 'a', created_at: '', updated_at: '' },
      ],
      error: undefined,
    });
    POST.mockResolvedValueOnce({ data: { id: 't1' }, error: undefined });

    const ok = await store.moveTaskToBoard('ws', 'AB-1', 'b2');

    expect(ok).toBe(true);
    // First column by position_key is 'c-a' (key 'a').
    expect(POST).toHaveBeenCalledWith('/v1/workspaces/{ws}/tasks/{readable_id}/move', {
      params: { path: { ws: 'ws', readable_id: 'AB-1' } },
      body: { column_id: 'c-a', before: null, after: null },
    });
  });

  it('moveTaskToBoard fails when the target board has no columns', async () => {
    const store = useBoardsStore();
    GET.mockResolvedValueOnce({ data: [], error: undefined });

    const ok = await store.moveTaskToBoard('ws', 'AB-1', 'b2');

    expect(ok).toBe(false);
    expect(store.error).toContain('no columns');
    expect(POST).not.toHaveBeenCalled();
  });

  it('duplicateTask copies title and description into the source column', async () => {
    const store = useBoardsStore();
    GET.mockResolvedValueOnce({
      data: {
        readable_id: 'AB-1',
        column_id: 'col-1',
        title: 'Original',
        description: 'Body',
        priority: null,
      },
      error: undefined,
    });
    POST.mockResolvedValueOnce({ data: { readable_id: 'AB-9' }, error: undefined });
    GET.mockResolvedValueOnce({ data: { items: [] }, error: undefined });

    const created = await store.duplicateTask('ws', 'board-1', 'AB-1');

    expect(created).toBe('AB-9');
    expect(POST).toHaveBeenCalledWith('/v1/workspaces/{ws}/boards/{board_id}/tasks', {
      params: { path: { ws: 'ws', board_id: 'board-1' } },
      body: { column_id: 'col-1', title: 'Original (copy)', description: 'Body' },
    });
  });
});
