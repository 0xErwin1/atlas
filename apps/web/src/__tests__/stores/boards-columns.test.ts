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

import type { ColumnDto } from '@/stores/boards';
import { useBoardsStore } from '@/stores/boards';

const col = (id: string, positionKey: string, color: string | null = null): ColumnDto => ({
  id,
  board_id: 'board-1',
  name: `Col ${id}`,
  position_key: positionKey,
  color,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
});

describe('useBoardsStore column mutations', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('updateColumn PATCHes the color (swatch id) and replaces the cached column', async () => {
    GET.mockResolvedValueOnce({ data: [col('c1', 'a'), col('c2', 'b')], error: undefined });
    PATCH.mockResolvedValueOnce({ data: col('c1', 'a', 'green'), error: undefined });

    const store = useBoardsStore();
    await store.loadColumns('ws', 'board-1');

    const ok = await store.updateColumn('ws', 'board-1', 'c1', { color: 'green' });

    expect(ok).toBe(true);
    expect(PATCH).toHaveBeenCalledWith('/api/workspaces/{ws}/boards/{board_id}/columns/{column_id}', {
      params: { path: { ws: 'ws', board_id: 'board-1', column_id: 'c1' } },
      body: { color: 'green' },
    });
    expect(store.columns.find((c) => c.id === 'c1')?.color).toBe('green');
  });

  it('updateColumn PATCHes a rename and updates the name in place', async () => {
    GET.mockResolvedValueOnce({ data: [col('c1', 'a')], error: undefined });
    PATCH.mockResolvedValueOnce({ data: { ...col('c1', 'a'), name: 'Renamed' }, error: undefined });

    const store = useBoardsStore();
    await store.loadColumns('ws', 'board-1');

    const ok = await store.updateColumn('ws', 'board-1', 'c1', { name: 'Renamed' });

    expect(ok).toBe(true);
    expect(PATCH).toHaveBeenCalledWith('/api/workspaces/{ws}/boards/{board_id}/columns/{column_id}', {
      params: { path: { ws: 'ws', board_id: 'board-1', column_id: 'c1' } },
      body: { name: 'Renamed' },
    });
    expect(store.columns.at(0)?.name).toBe('Renamed');
  });

  it('updateColumn returns false and sets error on failure', async () => {
    PATCH.mockResolvedValueOnce({ data: undefined, error: { hint: 'bad swatch' } });

    const store = useBoardsStore();
    const ok = await store.updateColumn('ws', 'board-1', 'c1', { color: 'nope' });

    expect(ok).toBe(false);
    expect(store.error).toBe('bad swatch');
  });

  it('deleteColumn DELETEs and removes the column from the cache', async () => {
    GET.mockResolvedValueOnce({ data: [col('c1', 'a'), col('c2', 'b')], error: undefined });
    DELETE.mockResolvedValueOnce({ error: undefined });

    const store = useBoardsStore();
    await store.loadColumns('ws', 'board-1');

    const ok = await store.deleteColumn('ws', 'board-1', 'c1');

    expect(ok).toBe(true);
    expect(DELETE).toHaveBeenCalledWith('/api/workspaces/{ws}/boards/{board_id}/columns/{column_id}', {
      params: { path: { ws: 'ws', board_id: 'board-1', column_id: 'c1' } },
    });
    expect(store.columns.map((c) => c.id)).toEqual(['c2']);
  });

  it('moveColumn PATCHes a reorder request and re-sorts by the returned position_key', async () => {
    GET.mockResolvedValueOnce({
      data: [col('c1', 'a'), col('c2', 'b'), col('c3', 'c')],
      error: undefined,
    });
    PATCH.mockResolvedValueOnce({ data: col('c1', 'd'), error: undefined });

    const store = useBoardsStore();
    await store.loadColumns('ws', 'board-1');

    const ok = await store.moveColumn('ws', 'board-1', 'c1', { before: 'c', after: null });

    expect(ok).toBe(true);
    expect(PATCH).toHaveBeenCalledWith('/api/workspaces/{ws}/boards/{board_id}/columns/{column_id}', {
      params: { path: { ws: 'ws', board_id: 'board-1', column_id: 'c1' } },
      body: { before: 'c', after: null },
    });
    expect(store.columns.map((c) => c.id)).toEqual(['c2', 'c3', 'c1']);
  });
});
