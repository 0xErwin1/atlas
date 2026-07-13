import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/api/wrapper', () => ({
  wrappedClient: {
    GET: vi.fn(),
    POST: vi.fn(),
    PATCH: vi.fn(),
    DELETE: vi.fn(),
  },
}));

vi.mock('@/lib/workspaceLiveUpdates', () => ({
  disposeWorkspaceLiveUpdates: vi.fn(),
  setWorkspaceLiveUpdatesAuthorizationInvalidator: vi.fn(),
}));

import { deferred } from '@/__tests__/deferred';
import { wrappedClient } from '@/api/wrapper';
import { disposeWorkspaceLiveUpdates } from '@/lib/workspaceLiveUpdates';
import type { MeResponse } from '@/stores/auth';
import { useAuthStore } from '@/stores/auth';
import { useLastViewedStore } from '@/stores/lastViewed';
import { useWorkspaceStore } from '@/stores/workspace';

const mockGet = wrappedClient.GET as ReturnType<typeof vi.fn>;
const mockPost = wrappedClient.POST as ReturnType<typeof vi.fn>;
const mockPatch = wrappedClient.PATCH as ReturnType<typeof vi.fn>;
const mockDelete = wrappedClient.DELETE as ReturnType<typeof vi.fn>;

const emptyProjects = { data: { items: [] }, error: undefined };

describe('useWorkspaceStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    try {
      localStorage.clear();
    } catch {
      // jsdom provides localStorage; ignore if absent
    }
  });

  it('starts with no active workspace', () => {
    const store = useWorkspaceStore();
    expect(store.activeWorkspaceSlug).toBeNull();
  });

  it('setActiveWorkspace updates slug (REQ-W12)', () => {
    const store = useWorkspaceStore();
    store.setActiveWorkspace('my-workspace');
    expect(store.activeWorkspaceSlug).toBe('my-workspace');
  });

  it('setActiveWorkspace replacing slug updates correctly', () => {
    const store = useWorkspaceStore();
    store.setActiveWorkspace('first');
    store.setActiveWorkspace('second');
    expect(store.activeWorkspaceSlug).toBe('second');
  });

  it('disposes live updates on a committed workspace switch, but not a transient null', () => {
    const store = useWorkspaceStore();
    store.setActiveWorkspace('first');
    vi.mocked(disposeWorkspaceLiveUpdates).mockClear();

    store.setActiveWorkspace(null);
    expect(disposeWorkspaceLiveUpdates).not.toHaveBeenCalled();

    store.switchWorkspace('second');
    expect(disposeWorkspaceLiveUpdates).toHaveBeenCalledOnce();
  });

  it('restores committed A when a pending switch to B transiently clears active before returning to A', () => {
    const store = useWorkspaceStore();
    store.setActiveWorkspace('workspace-a');
    store.projects = [{ slug: 'keep' }] as typeof store.projects;
    localStorage.setItem('atlas:workspace', 'workspace-a');
    vi.mocked(disposeWorkspaceLiveUpdates).mockClear();

    store.setActiveWorkspace(null);
    store.switchWorkspace('workspace-a');

    expect(store.activeWorkspaceSlug).toBe('workspace-a');
    expect(disposeWorkspaceLiveUpdates).not.toHaveBeenCalled();
    expect(store.projects).toEqual([{ slug: 'keep' }]);
    expect(localStorage.getItem('atlas:workspace')).toBe('workspace-a');
  });

  it('loadWorkspaces sets activeWorkspaceSlug to first returned workspace', async () => {
    mockGet.mockResolvedValueOnce({
      data: [
        {
          id: '00000000-0000-0000-0000-000000000001',
          name: 'Atlas',
          slug: 'atlas',
          created_at: '2024-01-01T00:00:00Z',
          updated_at: '2024-01-01T00:00:00Z',
        },
      ],
      error: undefined,
    });

    const store = useWorkspaceStore();
    const slug = await store.loadWorkspaces();

    expect(slug).toBe('atlas');
    expect(store.activeWorkspaceSlug).toBe('atlas');
    expect(store.workspaces).toHaveLength(1);
    expect(store.workspaces.at(0)?.slug).toBe('atlas');
  });

  it('loadWorkspaces returns null and does not set slug when list is empty', async () => {
    mockGet.mockResolvedValueOnce({ data: [], error: undefined });

    const store = useWorkspaceStore();
    const slug = await store.loadWorkspaces();

    expect(slug).toBeNull();
    expect(store.activeWorkspaceSlug).toBeNull();
  });

  it('loadWorkspaces returns null on API error without crashing', async () => {
    mockGet.mockResolvedValueOnce({
      data: undefined,
      error: { type: 'urn:atlas:error:unknown', title: 'Unauthorized', status: 401 },
    });

    const store = useWorkspaceStore();
    const slug = await store.loadWorkspaces();

    expect(slug).toBeNull();
    expect(store.activeWorkspaceSlug).toBeNull();
  });

  it('keeps members from the latest workspace load when responses resolve out of order', async () => {
    const firstResponse = deferred<{ data: { id: string }[]; error: undefined }>();
    const secondResponse = deferred<{ data: { id: string }[]; error: undefined }>();
    mockGet.mockImplementation((_path: string, request: { params: { path: { ws: string } } }) =>
      request.params.path.ws === 'workspace-a' ? firstResponse.promise : secondResponse.promise,
    );

    const store = useWorkspaceStore();
    store.setActiveWorkspace('workspace-a');
    const firstLoad = store.loadMembers('workspace-a');
    store.switchWorkspace('workspace-b');
    const secondLoad = store.loadMembers('workspace-b');

    secondResponse.resolve({ data: [{ id: 'member-b' }], error: undefined });
    await secondLoad;
    firstResponse.resolve({ data: [{ id: 'member-a' }], error: undefined });
    await firstLoad;

    expect(store.members.map((member) => member.id)).toEqual(['member-b']);
  });

  it('ignores a delayed member response after the active workspace changes', async () => {
    const response = deferred<{
      data: { id: string; display: string; principal_type: string; role: string }[];
      error: undefined;
    }>();
    mockGet.mockReturnValueOnce(response.promise);

    const auth = useAuthStore();
    auth.user = { id: 'member-a', username: 'user', is_root: false } as MeResponse;
    const store = useWorkspaceStore();
    store.setActiveWorkspace('workspace-a');
    const load = store.loadMembers('workspace-a');

    store.activeWorkspaceSlug = 'workspace-b';
    response.resolve({
      data: [{ id: 'member-a', display: 'User', principal_type: 'user', role: 'owner' }],
      error: undefined,
    });
    await load;

    expect(store.members).toEqual([]);
    expect(store.myWorkspaceRole).toBeNull();
  });

  it('settles member transport failures and publishes an error', async () => {
    mockGet.mockRejectedValueOnce(new Error('network unavailable'));

    const store = useWorkspaceStore();
    store.setActiveWorkspace('workspace-a');
    store.members = [{ id: 'stale-member' }] as typeof store.members;

    await expect(store.loadMembers('workspace-a')).resolves.toBeUndefined();

    expect(store.members).toEqual([]);
    expect(store.error).toBe('Failed to load members');
  });

  it('clears cached members only when the active workspace changes', () => {
    const auth = useAuthStore();
    auth.user = { id: 'member-a', username: 'user', is_root: false } as MeResponse;
    const store = useWorkspaceStore();
    store.activeWorkspaceSlug = 'workspace-a';
    store.members = [
      { id: 'member-a', display: 'User', principal_type: 'user', role: 'owner' },
    ] as typeof store.members;

    store.switchWorkspace('workspace-a');
    expect(store.members).toHaveLength(1);
    expect(store.myWorkspaceRole).toBe('owner');

    store.switchWorkspace('workspace-b');
    expect(store.members).toEqual([]);
    expect(store.myWorkspaceRole).toBeNull();
  });

  it('clears member state when deleting the active workspace selects a fallback', async () => {
    mockDelete.mockResolvedValueOnce({ error: undefined });
    const auth = useAuthStore();
    auth.user = { id: 'member-a', username: 'user', is_root: false } as MeResponse;
    const store = useWorkspaceStore();
    store.workspaces = [
      { id: '1', name: 'A', slug: 'workspace-a', created_at: 'x', updated_at: 'x' },
      { id: '2', name: 'B', slug: 'workspace-b', created_at: 'x', updated_at: 'x' },
    ];
    store.setActiveWorkspace('workspace-a');
    store.members = [
      { id: 'member-a', display: 'User', principal_type: 'user', role: 'owner' },
    ] as typeof store.members;

    await store.deleteWorkspace('workspace-a');

    expect(store.activeWorkspaceSlug).toBe('workspace-b');
    expect(store.members).toEqual([]);
    expect(store.myWorkspaceRole).toBeNull();
    expect(disposeWorkspaceLiveUpdates).toHaveBeenCalledOnce();
  });

  it('deleteWorkspace clears the deleted workspace last-viewed entry', async () => {
    mockDelete.mockResolvedValueOnce({ error: undefined });
    const store = useWorkspaceStore();
    store.workspaces = [
      { id: '1', name: 'A', slug: 'workspace-a', created_at: 'x', updated_at: 'x' },
      { id: '2', name: 'B', slug: 'workspace-b', created_at: 'x', updated_at: 'x' },
    ];
    store.setActiveWorkspace('workspace-a');

    const lastViewed = useLastViewedStore();
    lastViewed.record('workspace-a', { name: 'tasks', params: { boardId: 'board-1' } });
    lastViewed.record('workspace-b', { name: 'notes', params: { slug: 'keep' } });

    await store.deleteWorkspace('workspace-a');

    expect(lastViewed.forWorkspace('workspace-a')).toBeNull();
    expect(lastViewed.forWorkspace('workspace-b')).toEqual({ name: 'notes', params: { slug: 'keep' } });
  });

  it('updateWorkspaceSlug rekeys the active workspace last-viewed entry', async () => {
    mockPatch.mockResolvedValueOnce({
      data: { id: '1', name: 'Atlas', slug: 'atlas-new', created_at: 'x', updated_at: 'y' },
      error: undefined,
    });
    const store = useWorkspaceStore();
    store.workspaces = [{ id: '1', name: 'Atlas', slug: 'atlas', created_at: 'x', updated_at: 'x' }];
    store.setActiveWorkspace('atlas');

    const lastViewed = useLastViewedStore();
    lastViewed.record('atlas', { name: 'tasks', params: { boardId: 'board-1' } });

    const ok = await store.updateWorkspaceSlug('atlas', 'atlas-new');

    expect(ok).toBe(true);
    expect(store.activeWorkspaceSlug).toBe('atlas-new');
    expect(lastViewed.forWorkspace('atlas')).toBeNull();
    expect(lastViewed.forWorkspace('atlas-new')).toEqual({
      name: 'tasks',
      params: { boardId: 'board-1' },
    });
    expect(disposeWorkspaceLiveUpdates).toHaveBeenCalledOnce();
  });

  it('switching stays true until all overlapping switches settle, regardless of end order', () => {
    const store = useWorkspaceStore();

    const first = store.beginSwitch();
    const second = store.beginSwitch();
    expect(store.switching).toBe(true);

    // End the NEWEST token first: an in-flight count stays true here, whereas a
    // monotonic newest-token model would clear the flag on this first end.
    store.endSwitch(second);
    expect(store.switching).toBe(true);

    store.endSwitch(first);
    expect(store.switching).toBe(false);
  });

  it('isCurrentSwitch is true only for the latest overlapping switch token', () => {
    const store = useWorkspaceStore();

    const first = store.beginSwitch();
    expect(store.isCurrentSwitch(first)).toBe(true);

    const second = store.beginSwitch();
    expect(store.isCurrentSwitch(first)).toBe(false);
    expect(store.isCurrentSwitch(second)).toBe(true);
  });

  it('updateWorkspaceSlug rekeys a non-active workspace last-viewed entry', async () => {
    mockPatch.mockResolvedValueOnce({
      data: { id: '2', name: 'Other', slug: 'other-new', created_at: 'x', updated_at: 'y' },
      error: undefined,
    });
    const store = useWorkspaceStore();
    store.workspaces = [
      { id: '1', name: 'Atlas', slug: 'atlas', created_at: 'x', updated_at: 'x' },
      { id: '2', name: 'Other', slug: 'other', created_at: 'x', updated_at: 'x' },
    ];
    store.setActiveWorkspace('atlas');

    const lastViewed = useLastViewedStore();
    lastViewed.record('other', { name: 'notes', params: { slug: 'doc-1' } });

    const ok = await store.updateWorkspaceSlug('other', 'other-new');

    expect(ok).toBe(true);
    expect(store.activeWorkspaceSlug).toBe('atlas');
    expect(lastViewed.forWorkspace('other')).toBeNull();
    expect(lastViewed.forWorkspace('other-new')).toEqual({
      name: 'notes',
      params: { slug: 'doc-1' },
    });
  });

  it('renameProject PATCHes the project and refreshes the list', async () => {
    mockPatch.mockResolvedValueOnce({ error: undefined });
    mockGet.mockResolvedValueOnce(emptyProjects);

    const store = useWorkspaceStore();
    const ok = await store.renameProject('atlas', 'roadmap', 'Roadmap 2');

    expect(ok).toBe(true);
    expect(mockPatch).toHaveBeenCalledWith('/api/workspaces/{ws}/projects/{project_slug}', {
      params: { path: { ws: 'atlas', project_slug: 'roadmap' } },
      body: { name: 'Roadmap 2' },
    });
    expect(mockGet).toHaveBeenCalledWith('/api/workspaces/{ws}/projects', {
      params: { path: { ws: 'atlas' }, query: { limit: 200 } },
    });
  });

  it('renameProject returns false and sets error on API failure', async () => {
    mockPatch.mockResolvedValueOnce({
      error: { type: 'urn:atlas:error:forbidden', title: 'Forbidden', status: 403, hint: 'No permission' },
    });

    const store = useWorkspaceStore();
    const ok = await store.renameProject('atlas', 'roadmap', 'X');

    expect(ok).toBe(false);
    expect(store.error).toBe('No permission');
  });

  it('deleteProject DELETEs the project and refreshes the list', async () => {
    mockDelete.mockResolvedValueOnce({ error: undefined });
    mockGet.mockResolvedValueOnce(emptyProjects);

    const store = useWorkspaceStore();
    const ok = await store.deleteProject('atlas', 'roadmap');

    expect(ok).toBe(true);
    expect(mockDelete).toHaveBeenCalledWith('/api/workspaces/{ws}/projects/{project_slug}', {
      params: { path: { ws: 'atlas', project_slug: 'roadmap' } },
    });
    expect(mockGet).toHaveBeenCalledWith('/api/workspaces/{ws}/projects', {
      params: { path: { ws: 'atlas' }, query: { limit: 200 } },
    });
  });

  it('deleteProject returns false and sets error on API failure', async () => {
    mockDelete.mockResolvedValueOnce({
      error: { type: 'urn:atlas:error:forbidden', title: 'Forbidden', status: 403, hint: 'No permission' },
    });

    const store = useWorkspaceStore();
    const ok = await store.deleteProject('atlas', 'roadmap');

    expect(ok).toBe(false);
    expect(store.error).toBe('No permission');
  });

  it('loadWorkspaces restores the stored workspace when it still exists', async () => {
    localStorage.setItem('atlas:workspace', 'second');
    mockGet.mockResolvedValueOnce({
      data: [
        { id: '1', name: 'First', slug: 'first', created_at: 'x', updated_at: 'x' },
        { id: '2', name: 'Second', slug: 'second', created_at: 'x', updated_at: 'x' },
      ],
      error: undefined,
    });

    const store = useWorkspaceStore();
    const slug = await store.loadWorkspaces();

    expect(slug).toBe('second');
    expect(store.activeWorkspaceSlug).toBe('second');
  });

  it('switchWorkspace sets the slug, clears projects and persists', () => {
    const store = useWorkspaceStore();
    store.setActiveWorkspace('first');
    store.projects = [
      { slug: 'p', name: 'P', task_prefix: 'PRJ', workspace_id: 'w', visibility: 'workspace' },
    ];

    store.switchWorkspace('second');

    expect(store.activeWorkspaceSlug).toBe('second');
    expect(store.projects).toEqual([]);
    expect(localStorage.getItem('atlas:workspace')).toBe('second');
  });

  it('createWorkspace POSTs, refreshes the list and switches to the new one', async () => {
    mockPost.mockResolvedValueOnce({
      data: { id: '9', name: 'New WS', slug: 'new-ws', created_at: 'x', updated_at: 'x' },
      error: undefined,
    });
    mockGet.mockResolvedValueOnce({
      data: [{ id: '9', name: 'New WS', slug: 'new-ws', created_at: 'x', updated_at: 'x' }],
      error: undefined,
    });

    const store = useWorkspaceStore();
    const slug = await store.createWorkspace('New WS');

    expect(slug).toBe('new-ws');
    expect(mockPost).toHaveBeenCalledWith('/api/workspaces', { body: { name: 'New WS' } });
    expect(store.activeWorkspaceSlug).toBe('new-ws');
  });

  it('createWorkspace returns null and sets error on failure', async () => {
    mockPost.mockResolvedValueOnce({
      error: { type: 'urn:atlas:error:forbidden', title: 'Forbidden', status: 403, hint: 'Agents cannot' },
    });

    const store = useWorkspaceStore();
    const slug = await store.createWorkspace('Nope');

    expect(slug).toBeNull();
    expect(store.error).toBe('Agents cannot');
  });

  it('renameWorkspace PATCHes the workspace and reflects the new name in the cached list', async () => {
    mockPatch.mockResolvedValueOnce({
      data: { id: '1', name: 'Atlas Renamed', slug: 'atlas', created_at: 'x', updated_at: 'y' },
      error: undefined,
    });

    const store = useWorkspaceStore();
    store.workspaces = [{ id: '1', name: 'Atlas', slug: 'atlas', created_at: 'x', updated_at: 'x' }];

    const ok = await store.renameWorkspace('atlas', 'Atlas Renamed');

    expect(ok).toBe(true);
    expect(mockPatch).toHaveBeenCalledWith('/api/workspaces/{ws}', {
      params: { path: { ws: 'atlas' } },
      body: { name: 'Atlas Renamed' },
    });
    expect(store.workspaces.at(0)?.name).toBe('Atlas Renamed');
  });

  it('renameWorkspace returns false and sets error on API failure', async () => {
    mockPatch.mockResolvedValueOnce({
      data: undefined,
      error: { type: 'urn:atlas:error:forbidden', title: 'Forbidden', status: 403, hint: 'No permission' },
    });

    const store = useWorkspaceStore();
    const ok = await store.renameWorkspace('atlas', 'X');

    expect(ok).toBe(false);
    expect(store.error).toBe('No permission');
  });

  it('loadAdminWorkspaces GETs the admin list and stores it', async () => {
    mockGet.mockResolvedValueOnce({
      data: [
        { id: '1', name: 'Atlas', slug: 'atlas', created_at: 'x', updated_at: 'x' },
        { id: '2', name: 'Other', slug: 'other', created_at: 'x', updated_at: 'x' },
      ],
      error: undefined,
    });

    const store = useWorkspaceStore();
    await store.loadAdminWorkspaces();

    expect(mockGet).toHaveBeenCalledWith('/api/admin/workspaces');
    expect(store.adminWorkspaces).toHaveLength(2);
    expect(store.adminWorkspaces.at(1)?.slug).toBe('other');
  });

  it('loadAdminWorkspaces clears the list and sets error on failure', async () => {
    mockGet.mockResolvedValueOnce({
      data: undefined,
      error: { type: 'urn:atlas:error:forbidden', title: 'Forbidden', status: 403, hint: 'Root only' },
    });

    const store = useWorkspaceStore();
    await store.loadAdminWorkspaces();

    expect(store.adminWorkspaces).toEqual([]);
    expect(store.error).toBe('Root only');
  });
});
