import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET, PATCH, POST } = vi.hoisted(() => ({ GET: vi.fn(), PATCH: vi.fn(), POST: vi.fn() }));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET, PATCH, POST },
}));

import { useWorkspaceStore } from '@/stores/workspace';

function makePageResult(items: object[]) {
  return { data: { items, has_more: false, next_cursor: null }, error: undefined };
}

describe('useWorkspaceStore — ProjectSummary.task_prefix', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('loadProjects maps task_prefix into each ProjectSummary', async () => {
    GET.mockResolvedValueOnce(
      makePageResult([
        {
          id: 'p1',
          slug: 'atlas',
          name: 'Atlas',
          task_prefix: 'ATL',
          workspace_id: 'ws1',
          visibility: 'workspace',
          created_at: '',
          updated_at: '',
        },
      ]),
    );

    const store = useWorkspaceStore();
    await store.loadProjects('ws1');

    expect(store.projects).toHaveLength(1);
    expect(store.projects[0]?.task_prefix).toBe('ATL');
  });
});

describe('useWorkspaceStore — updateProject', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('sends name and task_prefix in the PATCH body and refreshes projects on success', async () => {
    PATCH.mockResolvedValueOnce({ error: undefined });
    GET.mockResolvedValueOnce(
      makePageResult([
        {
          id: 'p1',
          slug: 'atlas',
          name: 'Atlas Renamed',
          task_prefix: 'ATLX',
          workspace_id: 'ws1',
          visibility: 'workspace',
          created_at: '',
          updated_at: '',
        },
      ]),
    );

    const store = useWorkspaceStore();
    const ok = await store.updateProject('ws1', 'atlas', { name: 'Atlas Renamed', task_prefix: 'ATLX' });

    expect(ok).toBe(true);
    expect(PATCH).toHaveBeenCalledWith('/api/workspaces/{ws}/projects/{project_slug}', {
      params: { path: { ws: 'ws1', project_slug: 'atlas' } },
      body: { name: 'Atlas Renamed', task_prefix: 'ATLX' },
    });
    expect(store.projects[0]?.task_prefix).toBe('ATLX');
  });

  it('sends only name when task_prefix is not in the patch', async () => {
    PATCH.mockResolvedValueOnce({ error: undefined });
    GET.mockResolvedValueOnce(makePageResult([]));

    const store = useWorkspaceStore();
    const ok = await store.updateProject('ws1', 'atlas', { name: 'New Name' });

    expect(ok).toBe(true);
    expect(PATCH).toHaveBeenCalledWith('/api/workspaces/{ws}/projects/{project_slug}', {
      params: { path: { ws: 'ws1', project_slug: 'atlas' } },
      body: { name: 'New Name' },
    });
  });

  it('sets store error and returns false when the API returns an error', async () => {
    PATCH.mockResolvedValueOnce({ error: { hint: 'Prefix already used by another project' } });

    const store = useWorkspaceStore();
    const ok = await store.updateProject('ws1', 'atlas', { task_prefix: 'DUP' });

    expect(ok).toBe(false);
    expect(store.error).toBe('Prefix already used by another project');
  });
});

describe('useWorkspaceStore — loadAssignableUsers', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('populates assignableUsers from the GET response', async () => {
    GET.mockResolvedValueOnce({
      data: [
        { id: 'u1', username: 'alice', display_name: 'Alice', activated_at: '2024-01-01T00:00:00Z' },
        { id: 'u2', username: 'bob', display_name: 'Bob', activated_at: null },
      ],
      error: undefined,
    });

    const store = useWorkspaceStore();
    await store.loadAssignableUsers('ws1');

    expect(GET).toHaveBeenCalledWith('/api/workspaces/{ws}/assignable-users', {
      params: { path: { ws: 'ws1' } },
    });
    expect(store.assignableUsers).toHaveLength(2);
    expect(store.assignableUsers[0]?.username).toBe('alice');
  });

  it('clears assignableUsers when the API returns an error', async () => {
    GET.mockResolvedValueOnce({ data: undefined, error: { hint: 'forbidden' } });

    const store = useWorkspaceStore();
    store.assignableUsers = [
      { id: 'u9', username: 'stale', display_name: 'Stale', activated_at: null } as never,
    ];
    await store.loadAssignableUsers('ws1');

    expect(store.assignableUsers).toEqual([]);
  });
});

describe('useWorkspaceStore — addMember', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('posts user_id and role, and returns true on success', async () => {
    POST.mockResolvedValueOnce({ data: { id: 'u1' }, error: undefined });

    const store = useWorkspaceStore();
    const ok = await store.addMember('ws1', 'u1', 'admin');

    expect(ok).toBe(true);
    expect(POST).toHaveBeenCalledWith('/api/workspaces/{ws}/members', {
      params: { path: { ws: 'ws1' } },
      body: { user_id: 'u1', role: 'admin' },
    });
  });

  it('sets store error from the hint and returns false on failure', async () => {
    POST.mockResolvedValueOnce({ data: undefined, error: { hint: 'User is already a member' } });

    const store = useWorkspaceStore();
    const ok = await store.addMember('ws1', 'u1', 'member');

    expect(ok).toBe(false);
    expect(store.error).toBe('User is already a member');
  });
});
