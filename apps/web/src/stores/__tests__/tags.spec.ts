import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET, POST } = vi.hoisted(() => ({ GET: vi.fn(), POST: vi.fn() }));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET, POST },
}));

import { useTagsStore } from '@/stores/tags';

describe('useTagsStore — used labels', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('loadUsed populates usedLabels from the endpoint', async () => {
    GET.mockResolvedValueOnce({ data: ['urgent', 'backend', 'design'], error: undefined });

    const store = useTagsStore();
    await store.loadUsed('ws');

    expect(GET).toHaveBeenCalledWith('/api/workspaces/{ws}/tags/used', {
      params: { path: { ws: 'ws' } },
    });
    expect(store.usedLabels).toEqual(['urgent', 'backend', 'design']);
  });

  it('unregisteredLabels excludes case-insensitive matches of registered tags', () => {
    const store = useTagsStore();
    store.tags = [
      { id: 't1', name: 'Urgent', color: 'red' },
      { id: 't2', name: 'design', color: null },
    ] as never;
    store.usedLabels = ['urgent', 'BACKEND', 'Design', 'frontend'];

    expect(store.unregisteredLabels).toEqual(['BACKEND', 'frontend']);
  });

  it('registering a used label moves it out of unregisteredLabels', async () => {
    POST.mockResolvedValueOnce({
      data: { id: 't3', name: 'backend', color: null },
      error: undefined,
    });

    const store = useTagsStore();
    store.usedLabels = ['backend', 'frontend'];

    expect(store.unregisteredLabels).toEqual(['backend', 'frontend']);

    const created = await store.create('ws', 'backend');

    expect(created).not.toBeNull();
    expect(store.unregisteredLabels).toEqual(['frontend']);
  });
});
