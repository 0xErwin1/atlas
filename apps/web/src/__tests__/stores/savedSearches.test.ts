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

import { useSavedSearchesStore } from '@/stores/savedSearches';

const ss = (id: string, name: string, query = 'q') => ({
  id,
  name,
  query,
  workspace_id: 'ws-1',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
});

describe('useSavedSearchesStore (SE20)', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('load GETs the collection and fills items', async () => {
    GET.mockResolvedValueOnce({ data: [ss('s1', 'My search'), ss('s2', 'Another')], error: undefined });

    const store = useSavedSearchesStore();
    await store.load('ws-1');

    expect(GET).toHaveBeenCalledWith('/v1/workspaces/{ws}/saved-searches', {
      params: { path: { ws: 'ws-1' } },
    });
    expect(store.items).toHaveLength(2);
    expect(store.items[0]?.name).toBe('My search');
    expect(store.error).toBeNull();
  });

  it('load is a no-op on repeated calls for the same workspace', async () => {
    GET.mockResolvedValue({ data: [ss('s1', 'My search')], error: undefined });

    const store = useSavedSearchesStore();
    await store.load('ws-1');
    await store.load('ws-1');

    expect(GET).toHaveBeenCalledOnce();
  });

  it('load re-fetches when force=true', async () => {
    GET.mockResolvedValue({ data: [ss('s1', 'My search')], error: undefined });

    const store = useSavedSearchesStore();
    await store.load('ws-1');
    await store.load('ws-1', true);

    expect(GET).toHaveBeenCalledTimes(2);
  });

  it('load surfaces the API hint on error', async () => {
    GET.mockResolvedValueOnce({ data: undefined, error: { hint: 'Unauthorized' } });

    const store = useSavedSearchesStore();
    await store.load('ws-1');

    expect(store.error).toBe('Unauthorized');
    expect(store.items).toHaveLength(0);
  });

  it('create POSTs {name,query} and returns sorted DTO on success', async () => {
    POST.mockResolvedValueOnce({ data: ss('s1', 'Shell search', 'status:open'), error: undefined });

    const store = useSavedSearchesStore();
    const result = await store.create('ws-1', { name: 'Shell search', query: 'status:open' });

    expect(POST).toHaveBeenCalledWith('/v1/workspaces/{ws}/saved-searches', {
      params: { path: { ws: 'ws-1' } },
      body: { name: 'Shell search', query: 'status:open' },
    });
    expect(result).not.toBeNull();
    expect(result?.name).toBe('Shell search');
    expect(store.items).toHaveLength(1);
  });

  it('create returns null and sets error to hint on duplicate name (409 path, SE13)', async () => {
    POST.mockResolvedValueOnce({
      data: undefined,
      error: { hint: 'A saved search with this name already exists' },
    });

    const store = useSavedSearchesStore();
    const result = await store.create('ws-1', { name: 'Dup', query: 'q' });

    expect(result).toBeNull();
    expect(store.error).toBe('A saved search with this name already exists');
    expect(store.items).toHaveLength(0);
  });

  it('create returns null and sets error to hint on cap exceeded (422 path, SE14)', async () => {
    POST.mockResolvedValueOnce({ data: undefined, error: { hint: 'Per-owner saved search limit reached' } });

    const store = useSavedSearchesStore();
    const result = await store.create('ws-1', { name: 'Over cap', query: 'q' });

    expect(result).toBeNull();
    expect(store.error).toBe('Per-owner saved search limit reached');
  });

  it('create appends the new item keeping items name-sorted', async () => {
    POST.mockResolvedValueOnce({ data: ss('s2', 'Alpha'), error: undefined });

    const store = useSavedSearchesStore();
    store.items.push(ss('s1', 'Zeta'));

    const result = await store.create('ws-1', { name: 'Alpha', query: 'q' });

    expect(result).not.toBeNull();
    expect(store.items[0]?.name).toBe('Alpha');
    expect(store.items[1]?.name).toBe('Zeta');
  });

  it('rename PATCHes {name} and replaces the item in items (SE18)', async () => {
    const original = ss('s1', 'Old name');
    const renamed = { ...original, name: 'New name' };
    PATCH.mockResolvedValueOnce({ data: renamed, error: undefined });

    const store = useSavedSearchesStore();
    store.items.push(original);

    const ok = await store.rename('ws-1', 's1', 'New name');

    expect(ok).toBe(true);
    expect(PATCH).toHaveBeenCalledWith('/v1/workspaces/{ws}/saved-searches/{id}', {
      params: { path: { ws: 'ws-1', id: 's1' } },
      body: { name: 'New name' },
    });
    expect(store.items[0]?.name).toBe('New name');
  });

  it('rename returns false and surfaces hint on failure', async () => {
    PATCH.mockResolvedValueOnce({ data: undefined, error: { hint: 'Name already used' } });

    const store = useSavedSearchesStore();
    store.items.push(ss('s1', 'Old name'));

    const ok = await store.rename('ws-1', 's1', 'Dup');

    expect(ok).toBe(false);
    expect(store.error).toBe('Name already used');
    expect(store.items[0]?.name).toBe('Old name');
  });

  it('remove DELETEs and filters the item from items (SE19)', async () => {
    DELETE.mockResolvedValueOnce({ data: undefined, error: undefined });

    const store = useSavedSearchesStore();
    store.items.push(ss('s1', 'Alpha'), ss('s2', 'Beta'));

    const ok = await store.remove('ws-1', 's1');

    expect(ok).toBe(true);
    expect(DELETE).toHaveBeenCalledWith('/v1/workspaces/{ws}/saved-searches/{id}', {
      params: { path: { ws: 'ws-1', id: 's1' } },
    });
    expect(store.items).toHaveLength(1);
    expect(store.items[0]?.id).toBe('s2');
  });

  it('remove returns false and surfaces hint on failure', async () => {
    DELETE.mockResolvedValueOnce({ data: undefined, error: { hint: 'Not found' } });

    const store = useSavedSearchesStore();
    store.items.push(ss('s1', 'Alpha'));

    const ok = await store.remove('ws-1', 's1');

    expect(ok).toBe(false);
    expect(store.error).toBe('Not found');
    expect(store.items).toHaveLength(1);
  });
});
