import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';

export type SearchHitDto = components['schemas']['SearchHitDto'];
export type SearchKind = components['schemas']['SearchKindDto'];
export type SearchType = 'all' | 'note' | 'task';
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
export const useSearchStore = defineStore('search', () => {
  const query = ref('');
  const type = ref<SearchType>('all');
  const sort = ref<SearchSort>('relevance');

  const results = ref<SearchHitDto[]>([]);
  const cursor = ref<string | null>(null);
  const hasMore = ref(false);
  const loading = ref(false);
  const error = ref<string | null>(null);

  function setQuery(value: string): void {
    query.value = value;
  }

  function setType(value: SearchType): void {
    type.value = value;
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
          type: type.value,
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
    type,
    sort,
    results,
    cursor,
    hasMore,
    loading,
    error,
    setQuery,
    setType,
    setSort,
    clear,
    reset,
    runSearch,
    loadMore,
    _setForTest,
  };
});
