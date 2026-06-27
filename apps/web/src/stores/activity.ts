import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import { errorHint } from '@/lib/apiError';

export type ActivityEntryDto = components['schemas']['ActivityEntryDto'];

/** Actor-type filter for the workspace activity feed. `null` means "all". */
export type ActorFilter = 'user' | 'api_key' | null;

interface ActivityPage {
  items: ActivityEntryDto[];
  next_cursor?: string | null;
  has_more: boolean;
}

/**
 * Workspace activity store: the access-filtered, cursor-paginated feed backing
 * the Settings "Activity" panel. The server filters every entry by the caller's
 * task access, so the store never reasons about visibility — it only fetches,
 * appends, and exposes the current filter selection.
 *
 * Pagination mirrors the search store: a recent-first list with load-more,
 * forwarding the opaque server cursor verbatim. Changing any filter resets the
 * list and re-fetches from the first page.
 */
export const useActivityStore = defineStore('activity', () => {
  const entries = ref<ActivityEntryDto[]>([]);
  const cursor = ref<string | null>(null);
  const hasMore = ref(false);
  const loading = ref(false);
  const loadingMore = ref(false);
  const error = ref<string | null>(null);

  const actor = ref<ActorFilter>(null);
  const from = ref<string | null>(null);
  const to = ref<string | null>(null);

  function setActor(value: ActorFilter): void {
    actor.value = value;
  }

  function setRange(nextFrom: string | null, nextTo: string | null): void {
    from.value = nextFrom;
    to.value = nextTo;
  }

  function reset(): void {
    entries.value = [];
    cursor.value = null;
    hasMore.value = false;
    error.value = null;
  }

  function buildQuery(pageCursor?: string): Record<string, string> {
    const query: Record<string, string> = {};
    if (actor.value !== null) query.actor = actor.value;
    if (from.value !== null) query.from = from.value;
    if (to.value !== null) query.to = to.value;
    if (pageCursor !== undefined) query.cursor = pageCursor;
    return query;
  }

  async function fetchPage(ws: string, pageCursor?: string): Promise<ActivityPage | null> {
    const { data, error: apiError } = await wrappedClient.GET('/v1/workspaces/{ws}/activity', {
      params: {
        path: { ws },
        query: buildQuery(pageCursor),
      },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to load activity');
      return null;
    }

    return data as ActivityPage;
  }

  /**
   * Load the first page for the current workspace and filter selection,
   * replacing the list and resetting pagination.
   */
  async function load(ws: string): Promise<void> {
    if (ws === '') {
      reset();
      return;
    }

    loading.value = true;
    error.value = null;

    const page = await fetchPage(ws);

    loading.value = false;

    if (page === null) {
      entries.value = [];
      cursor.value = null;
      hasMore.value = false;
      return;
    }

    entries.value = page.items;
    cursor.value = page.next_cursor ?? null;
    hasMore.value = page.has_more;
  }

  /**
   * Append the next page using the stored cursor. No-op when there is no further
   * page or a load is already in flight. Entries are appended in server order —
   * the keyset cursor guarantees no gap or overlap.
   */
  async function loadMore(ws: string): Promise<void> {
    if (!hasMore.value || cursor.value === null || loadingMore.value) {
      return;
    }

    loadingMore.value = true;
    error.value = null;

    const page = await fetchPage(ws, cursor.value);

    loadingMore.value = false;

    if (page === null) return;

    entries.value = [...entries.value, ...page.items];
    cursor.value = page.next_cursor ?? null;
    hasMore.value = page.has_more;
  }

  function _setForTest(state: {
    entries?: ActivityEntryDto[];
    cursor?: string | null;
    hasMore?: boolean;
  }): void {
    if (state.entries !== undefined) entries.value = state.entries;
    if (state.cursor !== undefined) cursor.value = state.cursor;
    if (state.hasMore !== undefined) hasMore.value = state.hasMore;
  }

  return {
    entries,
    cursor,
    hasMore,
    loading,
    loadingMore,
    error,
    actor,
    from,
    to,
    setActor,
    setRange,
    reset,
    load,
    loadMore,
    _setForTest,
  };
});
