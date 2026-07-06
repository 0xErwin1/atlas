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

import { useStatusTemplatesStore } from '@/stores/statusTemplates';

function tpl(over: Record<string, unknown> = {}) {
  return {
    id: 't1',
    workspace_id: 'ws1',
    name: 'Todo',
    color: null,
    position_key: 'a',
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    ...over,
  };
}

describe('useStatusTemplatesStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('load fetches templates and sorts them by position_key', async () => {
    GET.mockResolvedValueOnce({
      data: [
        tpl({ id: 't2', name: 'Doing', position_key: 'b' }),
        tpl({ id: 't1', name: 'Todo', position_key: 'a' }),
      ],
      error: undefined,
    });

    const store = useStatusTemplatesStore();
    await store.load('acme');

    expect(GET).toHaveBeenCalledWith('/api/workspaces/{ws}/status-templates', {
      params: { path: { ws: 'acme' } },
    });
    expect(store.templates.map((t) => t.id)).toEqual(['t1', 't2']);
  });

  it('create posts the name appended after the last template and caches the result', async () => {
    const store = useStatusTemplatesStore();
    store.templates = [tpl({ id: 't1', position_key: 'a' })] as never;

    POST.mockResolvedValueOnce({
      data: tpl({ id: 't2', name: 'Doing', position_key: 'b' }),
      error: undefined,
    });

    const created = await store.create('acme', 'Doing');

    expect(POST).toHaveBeenCalledWith('/api/workspaces/{ws}/status-templates', {
      params: { path: { ws: 'acme' } },
      body: { name: 'Doing', before: 'a', after: null },
    });
    expect(created).not.toBeNull();
    expect(store.templates.map((t) => t.id)).toEqual(['t1', 't2']);
  });

  it('update patches name and color and replaces the cached template', async () => {
    const store = useStatusTemplatesStore();
    store.templates = [tpl({ id: 't1', name: 'Todo', position_key: 'a' })] as never;

    PATCH.mockResolvedValueOnce({
      data: tpl({ id: 't1', name: 'Backlog', color: '#1A2B3C', position_key: 'a' }),
      error: undefined,
    });

    const ok = await store.update('acme', 't1', { name: 'Backlog', color: '#1A2B3C' });

    expect(PATCH).toHaveBeenCalledWith('/api/workspaces/{ws}/status-templates/{template_id}', {
      params: { path: { ws: 'acme', template_id: 't1' } },
      body: { name: 'Backlog', color: '#1A2B3C' },
    });
    expect(ok).toBe(true);
    expect(store.templates[0]?.name).toBe('Backlog');
  });

  it('move patches before/after anchors and re-sorts', async () => {
    const store = useStatusTemplatesStore();
    store.templates = [tpl({ id: 't1', position_key: 'a' }), tpl({ id: 't2', position_key: 'b' })] as never;

    PATCH.mockResolvedValueOnce({
      data: tpl({ id: 't1', position_key: 'c' }),
      error: undefined,
    });

    const ok = await store.move('acme', 't1', { before: 'b', after: null });

    expect(PATCH).toHaveBeenCalledWith('/api/workspaces/{ws}/status-templates/{template_id}', {
      params: { path: { ws: 'acme', template_id: 't1' } },
      body: { before: 'b', after: null },
    });
    expect(ok).toBe(true);
    expect(store.templates.map((t) => t.id)).toEqual(['t2', 't1']);
  });

  it('remove deletes the template and drops it from the cache', async () => {
    const store = useStatusTemplatesStore();
    store.templates = [tpl({ id: 't1' }), tpl({ id: 't2' })] as never;

    DELETE.mockResolvedValueOnce({ error: undefined });

    const ok = await store.remove('acme', 't1');

    expect(DELETE).toHaveBeenCalledWith('/api/workspaces/{ws}/status-templates/{template_id}', {
      params: { path: { ws: 'acme', template_id: 't1' } },
    });
    expect(ok).toBe(true);
    expect(store.templates.map((t) => t.id)).toEqual(['t2']);
  });

  it('applyToBoard posts to the apply endpoint and returns true', async () => {
    const store = useStatusTemplatesStore();

    POST.mockResolvedValueOnce({ data: [], error: undefined });

    const ok = await store.applyToBoard('acme', 'board-1');

    expect(POST).toHaveBeenCalledWith('/api/workspaces/{ws}/boards/{board_id}/apply-status-templates', {
      params: { path: { ws: 'acme', board_id: 'board-1' } },
    });
    expect(ok).toBe(true);
  });

  it('sets error and returns false when an API call fails', async () => {
    const store = useStatusTemplatesStore();

    POST.mockResolvedValueOnce({ data: undefined, error: { hint: 'nope' } });

    const created = await store.create('acme', 'Doing');

    expect(created).toBeNull();
    expect(store.error).toBe('nope');
  });
});
