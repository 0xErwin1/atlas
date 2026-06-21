import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET } = vi.hoisted(() => ({
  GET: vi.fn(),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET },
}));

import { useSearchStore } from '@/stores/search';

const hit = (
  id: string,
  kind: 'document' | 'task',
  title: string,
  extra: Partial<{ snippet: string; readable_id: string; project_slug: string }> = {},
) => ({
  id,
  kind,
  title,
  score: 1,
  updated_at: '2026-01-01T00:00:00Z',
  ...extra,
});

const page = (items: ReturnType<typeof hit>[], next: string | null, hasMore: boolean) => ({
  data: { items, next_cursor: next, has_more: hasMore },
  error: undefined,
});

describe('useSearchStore (REQ-W23/W24)', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('runSearch sends q and sort (no type param) and replaces results (SE1, SE2)', async () => {
    GET.mockResolvedValueOnce(
      page([hit('d1', 'document', 'Shell'), hit('t1', 'task', 'ATL-42')], null, false),
    );

    const store = useSearchStore();
    store.setQuery('app rail');
    store.setSort('relevance');
    await store.runSearch('acme');

    expect(GET).toHaveBeenCalledWith('/v1/workspaces/{ws}/search', {
      params: { path: { ws: 'acme' }, query: { q: 'app rail', sort: 'relevance' } },
    });
    expect(store.results).toHaveLength(2);
    expect(store.results[0]?.kind).toBe('document');
    expect(store.results[1]?.readable_id).toBeUndefined();
  });

  it('passes filter tokens through verbatim in q (server parses them)', async () => {
    GET.mockResolvedValueOnce(page([], null, false));

    const store = useSearchStore();
    store.setQuery('status:open shell');
    await store.runSearch('acme');

    const call = GET.mock.calls[0]?.[1] as { params: { query: { q: string } } };
    expect(call.params.query.q).toBe('status:open shell');
  });

  it('does not call the API for a blank query and clears results', async () => {
    const store = useSearchStore();
    store._setForTest({ results: [hit('d1', 'document', 'old')], cursor: 'abc', hasMore: true });
    store.setQuery('   ');
    await store.runSearch('acme');

    expect(GET).not.toHaveBeenCalled();
    expect(store.results).toHaveLength(0);
    expect(store.hasMore).toBe(false);
  });

  it('stores the 34-char search cursor and forwards it verbatim on loadMore', async () => {
    const searchCursor = 'c'.repeat(34);
    GET.mockResolvedValueOnce(page([hit('d1', 'document', 'one')], searchCursor, true));

    const store = useSearchStore();
    store.setQuery('q');
    await store.runSearch('acme');

    expect(store.cursor).toBe(searchCursor);
    expect(store.hasMore).toBe(true);

    GET.mockResolvedValueOnce(page([hit('d2', 'document', 'two')], null, false));
    await store.loadMore('acme');

    const loadMoreCall = GET.mock.calls[1]?.[1] as { params: { query: { cursor?: string } } };
    expect(loadMoreCall.params.query.cursor).toBe(searchCursor);
    expect(store.results).toHaveLength(2);
    expect(store.results.map((r) => r.id)).toEqual(['d1', 'd2']);
    expect(store.hasMore).toBe(false);
  });

  it('loadMore is a no-op when there are no more pages', async () => {
    const store = useSearchStore();
    store._setForTest({ results: [hit('d1', 'document', 'one')], cursor: null, hasMore: false });
    store.setQuery('q');
    await store.loadMore('acme');

    expect(GET).not.toHaveBeenCalled();
    expect(store.results).toHaveLength(1);
  });

  it('handles an empty / no-match result set', async () => {
    GET.mockResolvedValueOnce(page([], null, false));

    const store = useSearchStore();
    store.setQuery('nomatch');
    await store.runSearch('acme');

    expect(store.results).toHaveLength(0);
    expect(store.hasMore).toBe(false);
    expect(store.error).toBeNull();
  });

  it('surfaces the API hint on error and leaves results empty', async () => {
    GET.mockResolvedValueOnce({ data: undefined, error: { hint: 'Search is temporarily unavailable' } });

    const store = useSearchStore();
    store.setQuery('q');
    await store.runSearch('acme');

    expect(store.error).toBe('Search is temporarily unavailable');
    expect(store.results).toHaveLength(0);
  });

  it('fetchPage does NOT send a ?type= query param (SE2)', async () => {
    GET.mockResolvedValueOnce(page([], null, false));

    const store = useSearchStore();
    store.setQuery('type:note shell');
    await store.runSearch('acme');

    const call = GET.mock.calls[0]?.[1] as { params: { query: Record<string, unknown> } };
    expect('type' in call.params.query).toBe(false);
    expect(call.params.query.q).toBe('type:note shell');
    expect(call.params.query.sort).toBeDefined();
  });

  it('SearchType and setType are not exported from the module (SE1)', async () => {
    const mod = await import('@/stores/search');
    expect('setType' in mod).toBe(false);
    expect('SearchType' in mod).toBe(false);
  });
});
