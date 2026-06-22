import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET, PATCH } = vi.hoisted(() => ({ GET: vi.fn(), PATCH: vi.fn() }));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET, PATCH },
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
    expect(PATCH).toHaveBeenCalledWith('/v1/workspaces/{ws}/projects/{project_slug}', {
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
    expect(PATCH).toHaveBeenCalledWith('/v1/workspaces/{ws}/projects/{project_slug}', {
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
