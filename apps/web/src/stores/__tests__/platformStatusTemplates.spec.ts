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

import { usePlatformStatusTemplatesStore } from '@/stores/platformStatusTemplates';

function tpl(over: Record<string, unknown> = {}) {
  return {
    id: 't1',
    name: 'Todo',
    color: null,
    position_key: 'a',
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    ...over,
  };
}

async function seeded(templates: object[]) {
  GET.mockResolvedValueOnce({ data: templates, error: undefined });
  const store = usePlatformStatusTemplatesStore();
  await store.load();
  return store;
}

describe('usePlatformStatusTemplatesStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('load hits the admin endpoint with no workspace and sorts by position_key', async () => {
    GET.mockResolvedValueOnce({
      data: [
        tpl({ id: 't2', name: 'Doing', position_key: 'b' }),
        tpl({ id: 't1', name: 'Todo', position_key: 'a' }),
      ],
      error: undefined,
    });

    const store = usePlatformStatusTemplatesStore();
    await store.load();

    expect(GET).toHaveBeenCalledWith('/api/admin/status-templates');
    expect(store.templates.map((t) => t.id)).toEqual(['t1', 't2']);
  });

  it('load surfaces a 403 as an error and leaves the cache empty', async () => {
    GET.mockResolvedValueOnce({
      data: undefined,
      error: { status: 403, hint: 'Admin access required' },
    });

    const store = usePlatformStatusTemplatesStore();
    await store.load();

    expect(store.error).not.toBeNull();
    expect(store.templates).toHaveLength(0);
  });

  it('create appends after the current last template', async () => {
    const store = await seeded([tpl({ id: 't1', position_key: 'a' })]);
    POST.mockResolvedValueOnce({
      data: tpl({ id: 't2', name: 'Done', position_key: 'b' }),
      error: undefined,
    });

    const created = await store.create('Done');

    expect(POST).toHaveBeenCalledWith('/api/admin/status-templates', {
      body: { name: 'Done', before: null, after: null },
    });
    expect(created?.id).toBe('t2');
    expect(store.templates.map((t) => t.id)).toEqual(['t1', 't2']);
  });

  it('update patches the row in place and keeps the position order', async () => {
    const store = await seeded([
      tpl({ id: 't1', position_key: 'a' }),
      tpl({ id: 't2', name: 'Doing', position_key: 'b' }),
    ]);
    PATCH.mockResolvedValueOnce({
      data: tpl({ id: 't1', name: 'Backlog', position_key: 'a' }),
      error: undefined,
    });

    const ok = await store.update('t1', { name: 'Backlog' });

    expect(ok).toBe(true);
    expect(PATCH).toHaveBeenCalledWith('/api/admin/status-templates/{template_id}', {
      params: { path: { template_id: 't1' } },
      body: { name: 'Backlog' },
    });
    expect(store.templates.map((t) => t.name)).toEqual(['Backlog', 'Doing']);
  });

  it('move re-sorts the cache by the returned position_key', async () => {
    const store = await seeded([
      tpl({ id: 't1', position_key: 'a' }),
      tpl({ id: 't2', name: 'Doing', position_key: 'b' }),
    ]);
    PATCH.mockResolvedValueOnce({
      data: tpl({ id: 't1', position_key: 'c' }),
      error: undefined,
    });

    const ok = await store.move('t1', { before: 'b', after: null });

    expect(ok).toBe(true);
    expect(store.templates.map((t) => t.id)).toEqual(['t2', 't1']);
  });

  it('remove drops the row from the cache', async () => {
    const store = await seeded([
      tpl({ id: 't1', position_key: 'a' }),
      tpl({ id: 't2', name: 'Doing', position_key: 'b' }),
    ]);
    DELETE.mockResolvedValueOnce({ error: undefined });

    const ok = await store.remove('t1');

    expect(ok).toBe(true);
    expect(DELETE).toHaveBeenCalledWith('/api/admin/status-templates/{template_id}', {
      params: { path: { template_id: 't1' } },
    });
    expect(store.templates.map((t) => t.id)).toEqual(['t2']);
  });

  it('a failed delete keeps the row and reports the error', async () => {
    const store = await seeded([tpl({ id: 't1', position_key: 'a' })]);
    DELETE.mockResolvedValueOnce({ error: { status: 404, hint: 'not found' } });

    const ok = await store.remove('t1');

    expect(ok).toBe(false);
    expect(store.error).not.toBeNull();
    expect(store.templates.map((t) => t.id)).toEqual(['t1']);
  });
});
