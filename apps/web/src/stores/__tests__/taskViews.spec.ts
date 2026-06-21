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

import { useTaskViewsStore } from '@/stores/taskViews';

const view = (id: string, name: string) => ({
  id,
  name,
  workspace_id: 'ws-1',
  filters: {},
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
});

describe('useTaskViewsStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('starts with empty items and no error', () => {
    const store = useTaskViewsStore();

    expect(store.items).toEqual([]);
    expect(store.error).toBeNull();
  });

  it('load fetches and stores views sorted by name', async () => {
    GET.mockResolvedValueOnce({
      data: [view('v2', 'Zeta'), view('v1', 'Alpha')],
      error: undefined,
    });

    const store = useTaskViewsStore();
    await store.load('ws');

    expect(store.items).toHaveLength(2);
    expect(store.items[0]?.name).toBe('Alpha');
    expect(store.items[1]?.name).toBe('Zeta');
  });

  it('load skips the request when workspace is unchanged (cache hit)', async () => {
    GET.mockResolvedValue({
      data: [],
      error: undefined,
    });

    const store = useTaskViewsStore();
    await store.load('ws');
    await store.load('ws');

    expect(GET).toHaveBeenCalledOnce();
  });

  it('load re-fetches when force is true', async () => {
    GET.mockResolvedValue({
      data: [],
      error: undefined,
    });

    const store = useTaskViewsStore();
    await store.load('ws');
    await store.load('ws', true);

    expect(GET).toHaveBeenCalledTimes(2);
  });

  it('load sets error on API failure', async () => {
    GET.mockResolvedValueOnce({
      data: undefined,
      error: { hint: 'Not found' },
    });

    const store = useTaskViewsStore();
    await store.load('ws');

    expect(store.error).toBe('Not found');
    expect(store.items).toEqual([]);
  });

  it('create POSTs and inserts the new view into the sorted list', async () => {
    GET.mockResolvedValueOnce({ data: [view('v1', 'Alpha')], error: undefined });
    POST.mockResolvedValueOnce({ data: view('v2', 'Mango'), error: undefined });

    const store = useTaskViewsStore();
    await store.load('ws');

    const created = await store.create('ws', { name: 'Mango', filters: {} });

    expect(created).not.toBeNull();
    expect(created?.id).toBe('v2');
    expect(store.items).toHaveLength(2);
    expect(store.items[0]?.name).toBe('Alpha');
    expect(store.items[1]?.name).toBe('Mango');
  });

  it('create returns null on error and sets error message', async () => {
    GET.mockResolvedValueOnce({ data: [], error: undefined });
    POST.mockResolvedValueOnce({ data: undefined, error: { hint: 'Name too long' } });

    const store = useTaskViewsStore();
    await store.load('ws');

    const result = await store.create('ws', { name: 'x'.repeat(101), filters: {} });

    expect(result).toBeNull();
    expect(store.error).toBe('Name too long');
  });

  it('update PATCHes and reflects the new name in the sorted list', async () => {
    GET.mockResolvedValueOnce({ data: [view('v1', 'Alpha')], error: undefined });
    PATCH.mockResolvedValueOnce({ data: view('v1', 'Zeta'), error: undefined });

    const store = useTaskViewsStore();
    await store.load('ws');

    const ok = await store.update('ws', 'v1', { name: 'Zeta', filters: {} });

    expect(ok).toBe(true);
    expect(store.items[0]?.name).toBe('Zeta');
  });

  it('update returns false on error', async () => {
    GET.mockResolvedValueOnce({ data: [view('v1', 'Alpha')], error: undefined });
    PATCH.mockResolvedValueOnce({ data: undefined, error: { hint: 'Conflict' } });

    const store = useTaskViewsStore();
    await store.load('ws');

    const ok = await store.update('ws', 'v1', { name: 'Zeta', filters: {} });

    expect(ok).toBe(false);
    expect(store.error).toBe('Conflict');
  });

  it('remove DELETEs and removes the view from the list', async () => {
    GET.mockResolvedValueOnce({ data: [view('v1', 'Alpha'), view('v2', 'Beta')], error: undefined });
    DELETE.mockResolvedValueOnce({ error: undefined });

    const store = useTaskViewsStore();
    await store.load('ws');

    const ok = await store.remove('ws', 'v1');

    expect(ok).toBe(true);
    expect(store.items).toHaveLength(1);
    expect(store.items[0]?.id).toBe('v2');
  });

  it('remove returns false on error', async () => {
    GET.mockResolvedValueOnce({ data: [view('v1', 'Alpha')], error: undefined });
    DELETE.mockResolvedValueOnce({ error: { hint: 'Not found' } });

    const store = useTaskViewsStore();
    await store.load('ws');

    const ok = await store.remove('ws', 'v1');

    expect(ok).toBe(false);
    expect(store.error).toBe('Not found');
    expect(store.items).toHaveLength(1);
  });
});
