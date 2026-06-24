import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';

export type AuditEntryDto = components['schemas']['AuditEntryDto'];

/** Actor-type filter for the security audit feed. `null` means "all". */
export type AuditActorFilter = 'user' | 'api_key' | null;

interface AuditPage {
  items: AuditEntryDto[];
  next_cursor?: string | null;
  has_more: boolean;
}

/**
 * Security audit store backing the workspace "Security log" and the platform
 * audit panels. Both feeds are recent-first and cursor-paginated; the server
 * already gates access (workspace owner/admin, or platform admin), so the store
 * only fetches, appends, and exposes the current filter selection.
 *
 * Two fetch entry points share one filter/pagination state: `loadWorkspace`
 * targets `/v1/workspaces/{ws}/audit`, `loadPlatform` targets `/v1/admin/audit`.
 * A panel uses exactly one of them and never switches scope mid-life, so they
 * can share the same backing refs. Changing any filter resets and re-fetches.
 */
export const useAuditStore = defineStore('audit', () => {
  const entries = ref<AuditEntryDto[]>([]);
  const cursor = ref<string | null>(null);
  const hasMore = ref(false);
  const loading = ref(false);
  const loadingMore = ref(false);
  const error = ref<string | null>(null);

  const actor = ref<AuditActorFilter>(null);
  const action = ref<string | null>(null);
  const from = ref<string | null>(null);
  const to = ref<string | null>(null);

  function setActor(value: AuditActorFilter): void {
    actor.value = value;
  }

  function setAction(value: string | null): void {
    action.value = value;
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
    if (action.value !== null) query.action = action.value;
    if (from.value !== null) query.from = from.value;
    if (to.value !== null) query.to = to.value;
    if (pageCursor !== undefined) query.cursor = pageCursor;
    return query;
  }

  async function fetchWorkspacePage(ws: string, pageCursor?: string): Promise<AuditPage | null> {
    const { data, error: apiError } = await wrappedClient.GET('/v1/workspaces/{ws}/audit', {
      params: {
        path: { ws },
        query: buildQuery(pageCursor),
      },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to load the security log';
      return null;
    }

    return data as AuditPage;
  }

  async function fetchPlatformPage(pageCursor?: string): Promise<AuditPage | null> {
    const { data, error: apiError } = await wrappedClient.GET('/v1/admin/audit', {
      params: {
        query: buildQuery(pageCursor),
      },
    });

    if (apiError !== undefined || data === undefined) {
      error.value =
        (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to load the platform audit log';
      return null;
    }

    return data as AuditPage;
  }

  function applyFirstPage(page: AuditPage | null): void {
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

  function applyNextPage(page: AuditPage | null): void {
    if (page === null) return;

    entries.value = [...entries.value, ...page.items];
    cursor.value = page.next_cursor ?? null;
    hasMore.value = page.has_more;
  }

  /** Load the first page of the workspace security log for the current filters. */
  async function loadWorkspace(ws: string): Promise<void> {
    if (ws === '') {
      reset();
      return;
    }

    loading.value = true;
    error.value = null;

    const page = await fetchWorkspacePage(ws);

    loading.value = false;
    applyFirstPage(page);
  }

  /** Append the next workspace page using the stored cursor. */
  async function loadMoreWorkspace(ws: string): Promise<void> {
    if (!hasMore.value || cursor.value === null || loadingMore.value || ws === '') {
      return;
    }

    loadingMore.value = true;
    error.value = null;

    const page = await fetchWorkspacePage(ws, cursor.value);

    loadingMore.value = false;
    applyNextPage(page);
  }

  /** Load the first page of the platform audit log for the current filters. */
  async function loadPlatform(): Promise<void> {
    loading.value = true;
    error.value = null;

    const page = await fetchPlatformPage();

    loading.value = false;
    applyFirstPage(page);
  }

  /** Append the next platform page using the stored cursor. */
  async function loadMorePlatform(): Promise<void> {
    if (!hasMore.value || cursor.value === null || loadingMore.value) {
      return;
    }

    loadingMore.value = true;
    error.value = null;

    const page = await fetchPlatformPage(cursor.value);

    loadingMore.value = false;
    applyNextPage(page);
  }

  function _setForTest(state: {
    entries?: AuditEntryDto[];
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
    action,
    from,
    to,
    setActor,
    setAction,
    setRange,
    reset,
    loadWorkspace,
    loadMoreWorkspace,
    loadPlatform,
    loadMorePlatform,
    _setForTest,
  };
});
