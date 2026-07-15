import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET } = vi.hoisted(() => ({ GET: vi.fn() }));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET },
}));

import { useWorkspaceTasksStore } from '@/stores/workspaceTasks';

const task = (id: string, readableId: string) => ({
  id,
  readable_id: readableId,
  column_id: 'col-1',
  board_name: 'Board',
  column_name: 'Todo',
  title: `Task ${id}`,
  priority: null,
  updated_at: '2026-01-01T00:00:00Z',
});

describe('useWorkspaceTasksStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('starts with empty tasks, not loading, no error', () => {
    const store = useWorkspaceTasksStore();

    expect(store.tasks).toEqual([]);
    expect(store.loading).toBe(false);
    expect(store.error).toBeNull();
    expect(store.hasMore).toBe(false);
  });

  it('load fetches tasks and populates state', async () => {
    GET.mockResolvedValueOnce({
      data: { items: [task('t1', 'ATL-1'), task('t2', 'ATL-2')], has_more: false, next_cursor: null },
      error: undefined,
    });

    const store = useWorkspaceTasksStore();
    await store.load('ws', {});

    expect(GET).toHaveBeenCalledOnce();
    expect(store.tasks).toHaveLength(2);
    expect(store.tasks[0]?.readable_id).toBe('ATL-1');
    expect(store.loading).toBe(false);
    expect(store.error).toBeNull();
    expect(store.hasMore).toBe(false);
  });

  it('load passes assignee param when provided', async () => {
    GET.mockResolvedValueOnce({
      data: { items: [], has_more: false, next_cursor: null },
      error: undefined,
    });

    const store = useWorkspaceTasksStore();
    await store.load('ws', { assignee: 'me' });

    expect(GET).toHaveBeenCalledWith('/api/workspaces/{ws}/tasks', {
      params: {
        path: { ws: 'ws' },
        query: expect.objectContaining({ assignee: 'me', limit: 200 }),
      },
    });
  });

  it('load passes sort param when provided', async () => {
    GET.mockResolvedValueOnce({
      data: { items: [], has_more: false, next_cursor: null },
      error: undefined,
    });

    const store = useWorkspaceTasksStore();
    await store.load('ws', { sort: 'updated_at_desc' });

    expect(GET).toHaveBeenCalledWith('/api/workspaces/{ws}/tasks', {
      params: {
        path: { ws: 'ws' },
        query: expect.objectContaining({ sort: 'updated_at_desc', limit: 200 }),
      },
    });
  });

  it('load surfaces has_more from the API', async () => {
    GET.mockResolvedValueOnce({
      data: { items: [task('t1', 'ATL-1')], has_more: true, next_cursor: 'cursor-xyz' },
      error: undefined,
    });

    const store = useWorkspaceTasksStore();
    await store.load('ws', {});

    expect(store.hasMore).toBe(true);
    expect(store.nextCursor).toBe('cursor-xyz');
  });

  it('load sets error on API failure', async () => {
    GET.mockResolvedValueOnce({
      data: undefined,
      error: { hint: 'Unauthorized' },
    });

    const store = useWorkspaceTasksStore();
    await store.load('ws', {});

    expect(store.tasks).toEqual([]);
    expect(store.error).toBe('Unauthorized');
  });

  it('load sets fallback error message when hint is absent', async () => {
    GET.mockResolvedValueOnce({
      data: undefined,
      error: {},
    });

    const store = useWorkspaceTasksStore();
    await store.load('ws', {});

    expect(store.error).toBe('Failed to load tasks');
  });

  it('re-fetches without an authoritative cache scope even when params are unchanged', async () => {
    GET.mockResolvedValue({
      data: { items: [], has_more: false, next_cursor: null },
      error: undefined,
    });

    const store = useWorkspaceTasksStore();
    await store.load('ws', { assignee: 'me' });
    await store.load('ws', { assignee: 'me' });

    expect(GET).toHaveBeenCalledTimes(2);
  });

  it('load re-fetches when force is true even with same params', async () => {
    GET.mockResolvedValue({
      data: { items: [], has_more: false, next_cursor: null },
      error: undefined,
    });

    const store = useWorkspaceTasksStore();
    await store.load('ws', {});
    await store.load('ws', {}, true);

    expect(GET).toHaveBeenCalledTimes(2);
  });

  it('load re-fetches when the workspace changes', async () => {
    GET.mockResolvedValue({
      data: { items: [], has_more: false, next_cursor: null },
      error: undefined,
    });

    const store = useWorkspaceTasksStore();
    await store.load('ws-a', {});
    await store.load('ws-b', {});

    expect(GET).toHaveBeenCalledTimes(2);
  });
});

describe('paramsForView', () => {
  it('maps my-tasks to assignee me', async () => {
    const { paramsForView } = await import('@/stores/workspaceTasks');
    expect(paramsForView('my-tasks')).toEqual({ assignee: 'me' });
  });

  it('maps recently-updated to sort updated_at_desc', async () => {
    const { paramsForView } = await import('@/stores/workspaceTasks');
    expect(paramsForView('recently-updated')).toEqual({ sort: 'updated_at_desc' });
  });

  it('maps agent-activity to actor api_key + sort', async () => {
    const { paramsForView } = await import('@/stores/workspaceTasks');
    expect(paramsForView('agent-activity')).toEqual({ actor: 'api_key', sort: 'updated_at_desc' });
  });

  it('maps a UUID to the custom view filters', async () => {
    const { paramsForView } = await import('@/stores/workspaceTasks');
    const filters = {
      assignee: 'me',
      actor_type: 'api_key',
      priorities: ['high', 'urgent'],
      labels: ['bug'],
      sort: 'priority_desc',
    };
    const result = paramsForView('some-uuid', filters);
    expect(result).toEqual({
      assignee: 'me',
      actor: 'api_key',
      priority: ['high', 'urgent'],
      label: ['bug'],
      sort: 'priority_desc',
    });
  });

  it('maps a UUID with empty filters to an empty param object', async () => {
    const { paramsForView } = await import('@/stores/workspaceTasks');
    expect(paramsForView('some-uuid', {})).toEqual({});
  });
});
