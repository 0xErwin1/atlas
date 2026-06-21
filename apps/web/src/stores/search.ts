import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';

export type SearchHitDto = components['schemas']['SearchHitDto'];
export type SearchKind = components['schemas']['SearchKindDto'];
export type SearchSort = 'relevance' | 'updated';

interface SearchPage {
  items: SearchHitDto[];
  next_cursor?: string | null;
  has_more: boolean;
}

/**
 * Search store (REQ-W23/W24): query state, type/sort filters, the ranked result
 * list, and cursor pagination over the E06 search endpoint.
 *
 * Server ranking is authoritative — results are stored in the order the API
 * returns them and never re-sorted client-side.
 *
 * The pagination cursor is the 34-char sort-aware search cursor (distinct from
 * the standard 22-char cursor); it is forwarded verbatim and never parsed.
 *
 * Filter tokens (status:, tag:, type:, …) ride inside the query string `q`; the
 * server parses them. The client only passes `q` through.
 */
const RECENTS_KEY = 'atlas:search-recents';

function loadRecents(): string[] {
  try {
    const raw = localStorage.getItem(RECENTS_KEY);
    if (raw !== null) return JSON.parse(raw) as string[];
  } catch {
    // ignore malformed storage
  }
  return [];
}

export const useSearchStore = defineStore('search', () => {
  const query = ref('');
  const sort = ref<SearchSort>('relevance');

  // The last few searches the user ran, most-recent first, for the sidebar's
  // Recent section. Persisted so it survives reloads.
  const recents = ref<string[]>(loadRecents());

  function recordRecent(value: string): void {
    const trimmed = value.trim();
    if (trimmed === '') return;

    recents.value = [trimmed, ...recents.value.filter((q) => q !== trimmed)].slice(0, 8);
    try {
      localStorage.setItem(RECENTS_KEY, JSON.stringify(recents.value));
    } catch {
      // ignore storage errors
    }
  }

  const results = ref<SearchHitDto[]>([]);
  const cursor = ref<string | null>(null);
  const hasMore = ref(false);
  const loading = ref(false);
  const error = ref<string | null>(null);

  function setQuery(value: string): void {
    query.value = value;
  }

  function setSort(value: SearchSort): void {
    sort.value = value;
  }

  function clear(): void {
    query.value = '';
    results.value = [];
    cursor.value = null;
    hasMore.value = false;
    error.value = null;
  }

  function reset(): void {
    results.value = [];
    cursor.value = null;
    hasMore.value = false;
    error.value = null;
  }

  async function fetchPage(ws: string, pageCursor?: string): Promise<SearchPage | null> {
    const { data, error: apiError } = await wrappedClient.GET('/v1/workspaces/{ws}/search', {
      params: {
        path: { ws },
        query: {
          q: query.value,
          sort: sort.value,
          ...(pageCursor !== undefined ? { cursor: pageCursor } : {}),
        },
      },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Search failed';
      return null;
    }

    return data as SearchPage;
  }

  /**
   * Run a fresh search for the current query, replacing the result list and
   * resetting pagination. A blank query is treated as "no search": the list is
   * cleared and no request is issued.
   */
  async function runSearch(ws: string): Promise<void> {
    if (query.value.trim() === '') {
      reset();
      return;
    }

    loading.value = true;
    error.value = null;

    const page = await fetchPage(ws);

    loading.value = false;

    if (page === null) {
      results.value = [];
      cursor.value = null;
      hasMore.value = false;
      return;
    }

    results.value = page.items;
    cursor.value = page.next_cursor ?? null;
    hasMore.value = page.has_more;
    recordRecent(query.value);
  }

  /**
   * Append the next page using the stored search cursor. No-op when there is no
   * further page. Results are appended in server order — no dedup or re-sort,
   * since the sort-aware cursor guarantees no gap or overlap.
   */
  async function loadMore(ws: string): Promise<void> {
    if (!hasMore.value || cursor.value === null) {
      return;
    }

    loading.value = true;
    error.value = null;

    const page = await fetchPage(ws, cursor.value);

    loading.value = false;

    if (page === null) {
      return;
    }

    results.value = [...results.value, ...page.items];
    cursor.value = page.next_cursor ?? null;
    hasMore.value = page.has_more;
  }

  function _setForTest(state: { results?: SearchHitDto[]; cursor?: string | null; hasMore?: boolean }): void {
    if (state.results !== undefined) results.value = state.results;
    if (state.cursor !== undefined) cursor.value = state.cursor;
    if (state.hasMore !== undefined) hasMore.value = state.hasMore;
  }

  return {
    query,
    sort,
    recents,
    results,
    cursor,
    hasMore,
    loading,
    error,
    setQuery,
    setSort,
    clear,
    reset,
    runSearch,
    loadMore,
    _setForTest,
  };
});
