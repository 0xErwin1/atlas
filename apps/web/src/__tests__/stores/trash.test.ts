import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET, POST } = vi.hoisted(() => ({ GET: vi.fn(), POST: vi.fn() }));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET, POST },
}));

import { useTrashStore } from '@/stores/trash';

const item = {
  kind: 'project' as const,
  target_id: '018f4abc-1234-7abc-8def-0123456789ab',
  workspace_id: '018f4abc-1234-7abc-8def-0123456789ac',
  deleted_at: '2026-07-22T00:00:00Z',
};

describe('trash store', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('loads a filtered page and keeps its cursor for the next page', async () => {
    GET.mockResolvedValueOnce({ data: { items: [item], has_more: true, next_cursor: 'next' } });
    const store = useTrashStore();

    await store.load({ workspaceId: item.workspace_id, kind: 'project' });

    expect(store.items).toEqual([item]);
    expect(store.nextCursor).toBe('next');
    expect(GET).toHaveBeenCalledWith('/api/admin/trash', {
      params: { query: { limit: 50, workspace_id: item.workspace_id, kind: 'project' } },
    });
  });

  it('restores an item and refreshes the current filter without navigating', async () => {
    GET.mockResolvedValueOnce({ data: { items: [item], has_more: false, next_cursor: null } });
    POST.mockResolvedValueOnce({ error: undefined });
    GET.mockResolvedValueOnce({ data: { items: [], has_more: false, next_cursor: null } });
    const store = useTrashStore();

    await store.load({ kind: 'project' });
    const restored = await store.restore(item);

    expect(restored).toBe(true);
    expect(store.items).toEqual([]);
    expect(POST).toHaveBeenCalledWith('/api/admin/trash/restore', {
      body: { kind: item.kind, target_id: item.target_id },
    });
  });

  it('keeps a pending purge operation and polls it to completion', async () => {
    POST.mockResolvedValueOnce({
      data: {
        ...item,
        operation_id: '018f4abc-1234-7abc-8def-0123456789ad',
        attempts: 1,
        status: 'cleanup_pending',
      },
      response: { status: 202 },
    });
    GET.mockResolvedValueOnce({
      data: {
        ...item,
        operation_id: '018f4abc-1234-7abc-8def-0123456789ad',
        attempts: 2,
        status: 'complete',
      },
    });
    GET.mockResolvedValueOnce({ data: { items: [], has_more: false, next_cursor: null } });
    const store = useTrashStore();

    const status = await store.purge(item);
    const completed = await store.poll(status?.operation_id ?? '');

    expect(status?.status).toBe('cleanup_pending');
    expect(completed?.status).toBe('complete');
    expect(POST).toHaveBeenCalledWith('/api/admin/trash/purge', {
      body: { kind: item.kind, target_id: item.target_id, confirm: true },
    });
  });

  it('clears stale rows immediately and ignores a late response for a previous filter', async () => {
    let resolveProjects:
      | ((value: { data: { items: (typeof item)[]; has_more: boolean; next_cursor: null } }) => void)
      | undefined;
    const projects = new Promise<{ data: { items: (typeof item)[]; has_more: boolean; next_cursor: null } }>(
      (resolve) => {
        resolveProjects = resolve;
      },
    );
    GET.mockReturnValueOnce(projects);
    GET.mockResolvedValueOnce({ data: { items: [], has_more: false, next_cursor: null } });
    const store = useTrashStore();

    const first = store.load({ kind: 'project' });
    expect(store.items).toEqual([]);

    await store.load({ kind: 'folder' });
    if (resolveProjects === undefined) throw new Error('Expected initial Trash request');
    resolveProjects({ data: { items: [item], has_more: false, next_cursor: null } });
    await first;

    expect(store.filter).toEqual({ kind: 'folder' });
    expect(store.items).toEqual([]);
  });

  it('keeps a failed poll retryable without publishing stale purge state', async () => {
    GET.mockResolvedValueOnce({ error: { hint: 'Retry later' } });
    const store = useTrashStore();

    const status = await store.poll('018f4abc-1234-7abc-8def-0123456789ad');

    expect(status).toBeNull();
    expect(store.error).toBe('Retry later');
  });
});
