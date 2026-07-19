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

  it('loadProjects maps task_prefix and visibility into each ProjectSummary', async () => {
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
    store.setActiveWorkspace('ws1');
    await store.loadProjects('ws1');

    expect(store.projects).toHaveLength(1);
    expect(store.projects[0]?.task_prefix).toBe('ATL');
    expect(store.projects[0]?.visibility).toBe('workspace');
  });
});

describe('useWorkspaceStore — loadProjects staleness and errors', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('clears the list locally for an empty slug without calling the API', async () => {
    const store = useWorkspaceStore();
    store.projects = [
      { slug: 's', name: 'S', task_prefix: 'S', workspace_id: 'ws1', visibility: 'workspace' },
    ];

    await store.loadProjects('');

    expect(GET).not.toHaveBeenCalled();
    expect(store.projects).toEqual([]);
    expect(store.projectsError).toBeNull();
  });

  it('does not let a late failure clobber a freshly loaded list', async () => {
    let resolveStale: (value: { data: undefined; error: { hint: string } }) => void = () => {};
    GET.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveStale = resolve;
      }),
    ).mockResolvedValueOnce(
      makePageResult([
        {
          id: 'p2',
          slug: 'fresh',
          name: 'Fresh',
          task_prefix: 'FR',
          workspace_id: 'ws1',
          visibility: 'workspace',
          created_at: '',
          updated_at: '',
        },
      ]),
    );

    const store = useWorkspaceStore();
    store.setActiveWorkspace('ws1');

    const stalePromise = store.loadProjects('ws1');
    const freshPromise = store.loadProjects('ws1');
    await freshPromise;

    expect(store.projects).toHaveLength(1);
    expect(store.projects[0]?.slug).toBe('fresh');

    resolveStale({ data: undefined, error: { hint: 'boom' } });
    await stalePromise;

    expect(store.projects).toHaveLength(1);
    expect(store.projects[0]?.slug).toBe('fresh');
    expect(store.projectsError).toBeNull();
  });

  it('sets projectsError and clears the list when the load fails', async () => {
    GET.mockResolvedValueOnce({ data: undefined, error: { hint: 'Service unavailable' } });

    const store = useWorkspaceStore();
    store.setActiveWorkspace('ws1');
    store.projects = [
      { slug: 's', name: 'S', task_prefix: 'S', workspace_id: 'ws1', visibility: 'workspace' },
    ];

    await store.loadProjects('ws1');

    expect(store.projects).toEqual([]);
    expect(store.projectsError).toBe('Service unavailable');
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
    store.setActiveWorkspace('ws1');
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

  it('sends visibility when changing project general access', async () => {
    PATCH.mockResolvedValueOnce({ error: undefined });
    GET.mockResolvedValueOnce(makePageResult([]));

    const store = useWorkspaceStore();
    const ok = await store.updateProject('ws1', 'atlas', { visibility: 'public' });

    expect(ok).toBe(true);
    expect(PATCH).toHaveBeenCalledWith('/api/workspaces/{ws}/projects/{project_slug}', {
      params: { path: { ws: 'ws1', project_slug: 'atlas' } },
      body: { visibility: 'public' },
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
