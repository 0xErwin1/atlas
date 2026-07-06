import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET } = vi.hoisted(() => ({
  GET: vi.fn(),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET },
}));

import { useActivityStore } from '@/stores/activity';

const entry = (id: string, kind = 'created') => ({
  id,
  kind,
  actor: { id: 'u1', type: 'user', display_name: 'Ada' },
  payload: kind,
  created_at: '2026-01-01T00:00:00Z',
  task_id: 't1',
  task_readable_id: 'ATL-1',
});

const page = (items: ReturnType<typeof entry>[], next: string | null, hasMore: boolean) => ({
  data: { items, next_cursor: next, has_more: hasMore },
  error: undefined,
});

describe('useActivityStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('load fetches the first page with no filter params and replaces entries', async () => {
    GET.mockResolvedValueOnce(page([entry('a1'), entry('a2')], 'cur1', true));

    const store = useActivityStore();
    await store.load('acme');

    expect(GET).toHaveBeenCalledWith('/api/workspaces/{ws}/activity', {
      params: { path: { ws: 'acme' }, query: {} },
    });
    expect(store.entries).toHaveLength(2);
    expect(store.cursor).toBe('cur1');
    expect(store.hasMore).toBe(true);
  });

  it('sends the actor filter when set', async () => {
    GET.mockResolvedValueOnce(page([], null, false));

    const store = useActivityStore();
    store.setActor('api_key');
    await store.load('acme');

    const call = GET.mock.calls[0]?.[1] as { params: { query: Record<string, string> } };
    expect(call.params.query.actor).toBe('api_key');
  });

  it('sends from/to date bounds when a range is set', async () => {
    GET.mockResolvedValueOnce(page([], null, false));

    const store = useActivityStore();
    store.setRange('2026-01-01T00:00:00.000Z', '2026-01-31T23:59:59.999Z');
    await store.load('acme');

    const call = GET.mock.calls[0]?.[1] as { params: { query: Record<string, string> } };
    expect(call.params.query.from).toBe('2026-01-01T00:00:00.000Z');
    expect(call.params.query.to).toBe('2026-01-31T23:59:59.999Z');
  });

  it('loadMore appends the next page using the stored cursor', async () => {
    const store = useActivityStore();
    store._setForTest({ entries: [entry('a1')], cursor: 'cur1', hasMore: true });

    GET.mockResolvedValueOnce(page([entry('a2')], null, false));
    await store.loadMore('acme');

    const call = GET.mock.calls[0]?.[1] as { params: { query: Record<string, string> } };
    expect(call.params.query.cursor).toBe('cur1');
    expect(store.entries.map((e) => e.id)).toEqual(['a1', 'a2']);
    expect(store.hasMore).toBe(false);
  });

  it('loadMore is a no-op when there is no further page', async () => {
    const store = useActivityStore();
    store._setForTest({ entries: [entry('a1')], cursor: null, hasMore: false });

    await store.loadMore('acme');

    expect(GET).not.toHaveBeenCalled();
  });

  it('surfaces the API hint in error on failure', async () => {
    GET.mockResolvedValueOnce({ data: undefined, error: { hint: 'nope' } });

    const store = useActivityStore();
    await store.load('acme');

    expect(store.error).toBe('nope');
    expect(store.entries).toHaveLength(0);
  });
});
