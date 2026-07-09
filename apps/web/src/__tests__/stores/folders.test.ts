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

import { useFoldersStore } from '@/stores/folders';

const folder = (id: string, name: string, parent: string | null = null) => ({
  id,
  name,
  parent_folder_id: parent,
  workspace_id: 'ws-1',
  project_id: 'p-1',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
});

describe('useFoldersStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('load populates folders from the project list (REQ-W14)', async () => {
    GET.mockResolvedValue({ data: { items: [folder('f1', 'Specs')], has_more: false } });

    const store = useFoldersStore();
    await store.load('ws', 'proj');

    expect(store.folders).toHaveLength(1);
    expect(store.folders[0]?.id).toBe('f1');
    expect(store.error).toBeNull();
  });

  it('load surfaces the API hint on error', async () => {
    GET.mockResolvedValue({ error: { hint: 'no access' } });

    const store = useFoldersStore();
    await store.load('ws', 'proj');

    expect(store.error).toBe('no access');
    expect(store.folders).toHaveLength(0);
  });

  it('keeps concurrent project folder loads isolated by project slug', async () => {
    let resolveAlpha: (value: { data: { items: ReturnType<typeof folder>[]; has_more: false } }) => void =
      () => {};
    GET.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveAlpha = resolve;
      }),
    );
    GET.mockResolvedValueOnce({ data: { items: [folder('b1', 'Beta')], has_more: false } });

    const store = useFoldersStore();
    const alphaLoad = store.load('ws', 'alpha');
    await store.load('ws', 'beta');
    resolveAlpha({ data: { items: [folder('a1', 'Alpha')], has_more: false } });
    await alphaLoad;

    expect(store.foldersFor('alpha').map((f) => f.id)).toEqual(['a1']);
    expect(store.foldersFor('beta').map((f) => f.id)).toEqual(['b1']);
    expect(store.folders.map((f) => f.id)).toEqual(['b1']);
  });

  it('refreshes only the owning project folders after a mutation', async () => {
    GET.mockResolvedValueOnce({ data: { items: [folder('a1', 'Alpha')], has_more: false } });
    GET.mockResolvedValueOnce({ data: { items: [folder('b1', 'Beta')], has_more: false } });
    PATCH.mockResolvedValueOnce({ data: folder('a1', 'Alpha Renamed') });
    GET.mockResolvedValueOnce({ data: { items: [folder('a1', 'Alpha Renamed')], has_more: false } });

    const store = useFoldersStore();
    await store.load('ws', 'alpha');
    await store.load('ws', 'beta');
    const ok = await store.rename('ws', 'alpha', 'a1', 'Alpha Renamed');

    expect(ok).toBe(true);
    expect(store.foldersFor('alpha').map((f) => f.name)).toEqual(['Alpha Renamed']);
    expect(store.foldersFor('beta').map((f) => f.name)).toEqual(['Beta']);
    expect(store.folders.map((f) => f.name)).toEqual(['Beta']);
    expect(GET.mock.calls[2]?.[1]?.params?.path?.project_slug).toBe('alpha');
  });

  it('load clears stale folders while loading a new project', async () => {
    let resolveLoad: (value: { data: { items: ReturnType<typeof folder>[]; has_more: false } }) => void =
      () => {};
    GET.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveLoad = resolve;
      }),
    );

    const store = useFoldersStore();
    store.$patch({ folders: [folder('old', 'Old')] });
    const pending = store.load('ws', 'next');

    expect(store.folders).toHaveLength(0);

    resolveLoad({ data: { items: [folder('new', 'New')], has_more: false } });
    await pending;
    expect(store.folders[0]?.id).toBe('new');
  });

  it('load ignores an older response after a newer load starts', async () => {
    let resolveFirst: (value: { data: { items: ReturnType<typeof folder>[]; has_more: false } }) => void =
      () => {};
    GET.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveFirst = resolve;
      }),
    );
    GET.mockResolvedValueOnce({ data: { items: [folder('new', 'New')], has_more: false } });

    const store = useFoldersStore();
    const first = store.load('ws', 'old');
    await store.load('ws', 'new');
    resolveFirst({ data: { items: [folder('old', 'Old')], has_more: false } });
    await first;

    expect(store.folders[0]?.id).toBe('new');
  });

  it('create refreshes silently without blanking the tree or toggling loading', async () => {
    POST.mockResolvedValueOnce({ data: folder('f2', 'New') });
    let resolveRefresh: (value: { data: { items: ReturnType<typeof folder>[]; has_more: false } }) => void =
      () => {};
    GET.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveRefresh = resolve;
      }),
    );

    const store = useFoldersStore();
    store.$patch({ folders: [folder('f1', 'Existing')] });

    const pending = store.create('ws', 'proj', 'New');

    expect(store.folders).toHaveLength(1);
    expect(store.folders[0]?.id).toBe('f1');
    expect(store.loading).toBe(false);

    resolveRefresh({ data: { items: [folder('f1', 'Existing'), folder('f2', 'New')], has_more: false } });
    await pending;

    expect(store.folders).toHaveLength(2);
    expect(store.loading).toBe(false);
  });

  it('releases the loader when a silent refresh supersedes an in-flight switch load', async () => {
    let resolveSwitch: (value: { data: { items: ReturnType<typeof folder>[]; has_more: false } }) => void =
      () => {};
    GET.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveSwitch = resolve;
      }),
    );
    POST.mockResolvedValueOnce({ data: folder('new', 'New') });
    GET.mockResolvedValueOnce({ data: { items: [folder('new', 'New')], has_more: false } });

    const store = useFoldersStore();
    const switchLoad = store.load('ws', 'proj');
    expect(store.loading).toBe(true);

    await store.create('ws', 'proj', 'New');

    resolveSwitch({ data: { items: [folder('old', 'Old')], has_more: false } });
    await switchLoad;

    expect(store.loading).toBe(false);
    expect(store.folders.map((f) => f.id)).toEqual(['new']);
  });

  it('create re-fetches the list on success', async () => {
    POST.mockResolvedValue({ data: folder('f2', 'New') });
    GET.mockResolvedValue({ data: { items: [folder('f2', 'New')], has_more: false } });

    const store = useFoldersStore();
    const ok = await store.create('ws', 'proj', 'New');

    expect(ok).toBe(true);
    expect(POST).toHaveBeenCalledOnce();
    expect(GET).toHaveBeenCalledOnce();
    expect(store.folders[0]?.id).toBe('f2');
  });

  it('create returns false and surfaces hint on failure', async () => {
    POST.mockResolvedValue({ error: { hint: 'name taken' } });

    const store = useFoldersStore();
    const ok = await store.create('ws', 'proj', 'Dup');

    expect(ok).toBe(false);
    expect(store.error).toBe('name taken');
    expect(GET).not.toHaveBeenCalled();
  });

  it('rename PATCHes and re-fetches', async () => {
    PATCH.mockResolvedValue({ data: folder('f1', 'Renamed') });
    GET.mockResolvedValue({ data: { items: [folder('f1', 'Renamed')], has_more: false } });

    const store = useFoldersStore();
    const ok = await store.rename('ws', 'proj', 'f1', 'Renamed');

    expect(ok).toBe(true);
    expect(PATCH).toHaveBeenCalledOnce();
  });

  it('remove DELETEs and re-fetches', async () => {
    DELETE.mockResolvedValue({ data: undefined });
    GET.mockResolvedValue({ data: { items: [], has_more: false } });

    const store = useFoldersStore();
    const ok = await store.remove('ws', 'proj', 'f1');

    expect(ok).toBe(true);
    expect(DELETE).toHaveBeenCalledOnce();
    expect(store.folders).toHaveLength(0);
  });

  it('move PATCHes the folder under a new parent and re-fetches', async () => {
    PATCH.mockResolvedValue({ error: undefined });
    GET.mockResolvedValue({ data: { items: [], has_more: false } });

    const store = useFoldersStore();
    const ok = await store.move('ws', 'proj', 'f1', 'parent-1');

    expect(ok).toBe(true);
    expect(PATCH).toHaveBeenCalledWith('/api/workspaces/{ws}/folders/{folder_id}/move', {
      params: { path: { ws: 'ws', folder_id: 'f1' } },
      body: { parent_folder_id: 'parent-1' },
    });
  });

  it('move with null parent targets the project root', async () => {
    PATCH.mockResolvedValue({ error: undefined });
    GET.mockResolvedValue({ data: { items: [], has_more: false } });

    const store = useFoldersStore();
    await store.move('ws', 'proj', 'f1', null);

    expect(PATCH).toHaveBeenCalledWith('/api/workspaces/{ws}/folders/{folder_id}/move', {
      params: { path: { ws: 'ws', folder_id: 'f1' } },
      body: { parent_folder_id: null },
    });
  });

  it('move returns false and surfaces hint on failure', async () => {
    PATCH.mockResolvedValue({ error: { hint: 'cycle' } });

    const store = useFoldersStore();
    const ok = await store.move('ws', 'proj', 'f1', 'parent-1');

    expect(ok).toBe(false);
    expect(store.error).toBe('cycle');
    expect(GET).not.toHaveBeenCalled();
  });

  it('copy POSTs to the copy endpoint and re-fetches', async () => {
    POST.mockResolvedValue({ data: folder('f9', 'Specs (copy)') });
    GET.mockResolvedValue({ data: { items: [], has_more: false } });

    const store = useFoldersStore();
    const ok = await store.copy('ws', 'proj', 'f1', 'parent-1');

    expect(ok).toBe(true);
    expect(POST).toHaveBeenCalledWith('/api/workspaces/{ws}/folders/{folder_id}/copy', {
      params: { path: { ws: 'ws', folder_id: 'f1' } },
      body: { parent_folder_id: 'parent-1' },
    });
  });

  it('copy returns false and surfaces hint on failure', async () => {
    POST.mockResolvedValue({ error: { hint: 'denied' } });

    const store = useFoldersStore();
    const ok = await store.copy('ws', 'proj', 'f1', null);

    expect(ok).toBe(false);
    expect(store.error).toBe('denied');
    expect(GET).not.toHaveBeenCalled();
  });
});
