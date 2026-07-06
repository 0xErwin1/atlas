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
  subtask_count: 0,
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
    expect(PATCH).toHaveBeenCalledWith('/api/workspaces/{ws}/tasks/{readable_id}', {
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

  it('assignTask succeeds and leaves both error channels clear', async () => {
    const store = useBoardsStore();
    POST.mockResolvedValueOnce({ error: undefined });

    const ok = await store.assignTask('ws', 'AB-1', 'user', 'u1');

    expect(ok).toBe(true);
    expect(store.error).toBeNull();
    expect(store.loadError).toBeNull();
  });

  it('assignTask 409 sets the action error, never loadError, so the board stays visible', async () => {
    const store = useBoardsStore();
    POST.mockResolvedValueOnce({ error: { status: 409, hint: 'Already assigned to this task' } });

    const ok = await store.assignTask('ws', 'AB-1', 'user', 'u1');

    expect(ok).toBe(false);
    expect(store.error).toBe('Already assigned to this task');
    expect(store.loadError).toBeNull();
  });

  it('assignTask 422 surfaces the actionable detail for a deactivated user', async () => {
    const store = useBoardsStore();
    POST.mockResolvedValueOnce({
      error: {
        status: 422,
        detail: 'Cannot assign a deactivated user; re-enable the account first.',
      },
    });

    const ok = await store.assignTask('ws', 'AB-1', 'user', 'u1');

    expect(ok).toBe(false);
    expect(store.error).toBe('Cannot assign a deactivated user; re-enable the account first.');
    expect(store.loadError).toBeNull();
  });

  it('unassignTask DELETEs the principal ref and clears both error channels', async () => {
    const store = useBoardsStore();
    DELETE.mockResolvedValueOnce({ error: undefined });

    const ok = await store.unassignTask('ws', 'AB-1', 'user', 'u1');

    expect(ok).toBe(true);
    expect(DELETE).toHaveBeenCalledWith('/api/workspaces/{ws}/tasks/{readable_id}/assignees/{assignee_ref}', {
      params: { path: { ws: 'ws', readable_id: 'AB-1', assignee_ref: 'user:u1' } },
    });
    expect(store.error).toBeNull();
    expect(store.loadError).toBeNull();
  });

  it('unassignTask sets the action error, never loadError, on failure', async () => {
    const store = useBoardsStore();
    DELETE.mockResolvedValueOnce({ error: { hint: 'nope' } });

    const ok = await store.unassignTask('ws', 'AB-1', 'api_key', 'a1');

    expect(ok).toBe(false);
    expect(store.error).toBe('nope');
    expect(store.loadError).toBeNull();
  });

  it('moveTaskToColumn posts a move with null neighbours', async () => {
    const store = useBoardsStore();
    POST.mockResolvedValueOnce({ data: { id: 't1' }, error: undefined });

    const ok = await store.moveTaskToColumn('ws', 'AB-1', 'col-2');

    expect(ok).toBe(true);
    expect(POST).toHaveBeenCalledWith('/api/workspaces/{ws}/tasks/{readable_id}/move', {
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
    expect(POST).toHaveBeenCalledWith('/api/workspaces/{ws}/tasks/{readable_id}/move', {
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
    expect(POST).toHaveBeenCalledWith('/api/workspaces/{ws}/boards/{board_id}/tasks', {
      params: { path: { ws: 'ws', board_id: 'board-1' } },
      body: { column_id: 'col-1', title: 'Original (copy)', description: 'Body' },
    });
  });

  it('createTask posts the column and title, reloads, and returns the readable id', async () => {
    const store = useBoardsStore();

    POST.mockResolvedValueOnce({ data: { readable_id: 'AB-7' }, error: undefined });
    GET.mockResolvedValueOnce({ data: { items: [] }, error: undefined });

    const created = await store.createTask('ws', 'board-1', 'col-2', 'New task');

    expect(created).toBe('AB-7');
    expect(POST).toHaveBeenCalledWith('/api/workspaces/{ws}/boards/{board_id}/tasks', {
      params: { path: { ws: 'ws', board_id: 'board-1' } },
      body: { column_id: 'col-2', title: 'New task' },
    });
    // Reloads the board's tasks so the new task appears in the list.
    expect(GET).toHaveBeenCalled();
  });

  it('createTask returns null and sets an error hint on failure', async () => {
    const store = useBoardsStore();

    POST.mockResolvedValueOnce({ data: undefined, error: { hint: 'Boom' } });

    const created = await store.createTask('ws', 'board-1', 'col-2', 'New task');

    expect(created).toBeNull();
    expect(store.error).toBe('Boom');
  });
});
