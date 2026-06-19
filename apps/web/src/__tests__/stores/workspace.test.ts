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

import { wrappedClient } from '@/api/wrapper';
import { useWorkspaceStore } from '@/stores/workspace';

const mockGet = wrappedClient.GET as ReturnType<typeof vi.fn>;
const mockPatch = wrappedClient.PATCH as ReturnType<typeof vi.fn>;
const mockDelete = wrappedClient.DELETE as ReturnType<typeof vi.fn>;

const emptyProjects = { data: { items: [] }, error: undefined };

describe('useWorkspaceStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
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

  it('renameProject PATCHes the project and refreshes the list', async () => {
    mockPatch.mockResolvedValueOnce({ error: undefined });
    mockGet.mockResolvedValueOnce(emptyProjects);

    const store = useWorkspaceStore();
    const ok = await store.renameProject('atlas', 'roadmap', 'Roadmap 2');

    expect(ok).toBe(true);
    expect(mockPatch).toHaveBeenCalledWith('/v1/workspaces/{ws}/projects/{project_slug}', {
      params: { path: { ws: 'atlas', project_slug: 'roadmap' } },
      body: { name: 'Roadmap 2' },
    });
    expect(mockGet).toHaveBeenCalledWith('/v1/workspaces/{ws}/projects', {
      params: { path: { ws: 'atlas' } },
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
    expect(mockDelete).toHaveBeenCalledWith('/v1/workspaces/{ws}/projects/{project_slug}', {
      params: { path: { ws: 'atlas', project_slug: 'roadmap' } },
    });
    expect(mockGet).toHaveBeenCalledWith('/v1/workspaces/{ws}/projects', {
      params: { path: { ws: 'atlas' } },
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
});
