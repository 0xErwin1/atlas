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
import { useTagsStore } from '@/stores/tags';

const mockGet = wrappedClient.GET as ReturnType<typeof vi.fn>;
const mockPost = wrappedClient.POST as ReturnType<typeof vi.fn>;
const mockPatch = wrappedClient.PATCH as ReturnType<typeof vi.fn>;
const mockDelete = wrappedClient.DELETE as ReturnType<typeof vi.fn>;

const tag = (id: string, name: string, color: string | null = null) => ({
  id,
  workspace_id: 'ws',
  name,
  color,
  created_at: 'x',
  updated_at: 'x',
});

describe('useTagsStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('load fetches tags for the workspace', async () => {
    mockGet.mockResolvedValueOnce({ data: [tag('1', 'bug'), tag('2', 'chore')], error: undefined });

    const store = useTagsStore();
    await store.load('ws');

    expect(mockGet).toHaveBeenCalledWith('/v1/workspaces/{ws}/tags', {
      params: { path: { ws: 'ws' } },
    });
    expect(store.tags).toHaveLength(2);
  });

  it('update PATCHes name and color and replaces the cached tag', async () => {
    mockGet.mockResolvedValueOnce({ data: [tag('1', 'bug')], error: undefined });
    mockPatch.mockResolvedValueOnce({ data: tag('1', 'defect', 'red'), error: undefined });

    const store = useTagsStore();
    await store.load('ws');

    const ok = await store.update('ws', '1', { name: 'defect', color: 'red' });

    expect(ok).toBe(true);
    expect(mockPatch).toHaveBeenCalledWith('/v1/workspaces/{ws}/tags/{tag_id}', {
      params: { path: { ws: 'ws', tag_id: '1' } },
      body: { name: 'defect', color: 'red' },
    });
    expect(store.tags.at(0)?.name).toBe('defect');
    expect(store.tags.at(0)?.color).toBe('red');
  });

  it('update returns false and sets error on API failure', async () => {
    mockPatch.mockResolvedValueOnce({
      data: undefined,
      error: { type: 'urn:atlas:error:conflict', title: 'Conflict', status: 409, hint: 'Name taken' },
    });

    const store = useTagsStore();
    const ok = await store.update('ws', '1', { name: 'dup' });

    expect(ok).toBe(false);
    expect(store.error).toBe('Name taken');
  });

  it('remove DELETEs the tag and drops it from the cache', async () => {
    mockGet.mockResolvedValueOnce({ data: [tag('1', 'bug'), tag('2', 'chore')], error: undefined });
    mockDelete.mockResolvedValueOnce({ error: undefined });

    const store = useTagsStore();
    await store.load('ws');

    const ok = await store.remove('ws', '1');

    expect(ok).toBe(true);
    expect(mockDelete).toHaveBeenCalledWith('/v1/workspaces/{ws}/tags/{tag_id}', {
      params: { path: { ws: 'ws', tag_id: '1' } },
    });
    expect(store.tags.map((t) => t.id)).toEqual(['2']);
  });

  it('remove returns false and sets error on API failure', async () => {
    mockDelete.mockResolvedValueOnce({
      error: { type: 'urn:atlas:error:forbidden', title: 'Forbidden', status: 403, hint: 'No permission' },
    });

    const store = useTagsStore();
    const ok = await store.remove('ws', '1');

    expect(ok).toBe(false);
    expect(store.error).toBe('No permission');
  });

  it('create POSTs a tag with name and color and caches it', async () => {
    mockPost.mockResolvedValueOnce({ data: tag('9', 'urgent', 'amber'), error: undefined });

    const store = useTagsStore();
    const created = await store.create('ws', 'urgent', 'amber');

    expect(created?.id).toBe('9');
    expect(mockPost).toHaveBeenCalledWith('/v1/workspaces/{ws}/tags', {
      params: { path: { ws: 'ws' } },
      body: { name: 'urgent', color: 'amber' },
    });
    expect(store.tags.at(0)?.name).toBe('urgent');
  });
});
