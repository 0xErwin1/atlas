import { defineStore } from 'pinia';
import { ref, watch } from 'vue';
import { z } from 'zod';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import {
  getResourceCachePrincipal,
  hydrateAndRevalidateResource,
  invalidateWorkspaceTaskQueryCache,
  resourceCache,
  resourceCacheEpoch,
} from '@/cache/cacheRuntime';
import { buildCacheKey, CACHE_CADENCE } from '@/cache/resourceCache';
import { errorHint } from '@/lib/apiError';

export type TaskSummaryDto = components['schemas']['TaskSummaryDto'];
export type TaskViewFiltersDto = components['schemas']['TaskViewFiltersDto'];

export interface WorkspaceTaskParams {
  assignee?: string;
  actor?: string;
  column_id?: string | string[];
  priority?: string | string[];
  label?: string | string[];
  board_id?: string;
  sort?: string;
  cursor?: string;
}

type WorkspaceTaskPage = {
  items: TaskSummaryDto[];
  has_more: boolean;
  next_cursor: string | null;
};

type WorkspaceTaskLoadError = Error & { hint: string; status?: number };

function taskLoadError(cause: unknown): WorkspaceTaskLoadError {
  const error = new Error(errorHint(cause, 'Failed to load tasks')) as WorkspaceTaskLoadError;
  error.hint = error.message;
  error.status = (cause as { status?: number } | undefined)?.status;
  return error;
}

function taskErrorHint(cause: unknown): string {
  return cause instanceof Error && 'hint' in cause && typeof cause.hint === 'string'
    ? cause.hint
    : errorHint(cause, 'Failed to load tasks');
}

function workspaceTaskPageSchema(params: WorkspaceTaskParams): z.ZodType<WorkspaceTaskPage> {
  return z
    .object({
      items: z.array(
        z
          .object({
            board_id: z.string().min(1),
            board_name: z.string(),
            column_id: z.string().min(1),
            column_name: z.string(),
            id: z.string().min(1),
            priority: z.string().nullable(),
            readable_id: z.string(),
            subtask_count: z.number(),
            title: z.string(),
            updated_at: z.string(),
          })
          .passthrough(),
      ),
      has_more: z.boolean(),
      next_cursor: z.string().nullable(),
    })
    .superRefine((page, context) => {
      if (params.board_id === undefined) return;

      for (const [index, task] of page.items.entries()) {
        if (task.board_id !== params.board_id) {
          context.addIssue({
            code: z.ZodIssueCode.custom,
            path: ['items', index, 'board_id'],
            message: 'Board mismatch',
          });
        }
      }
    });
}

/**
 * Translates a view ID (predefined slug or custom UUID) and optional custom
 * filters into the flat query params accepted by GET /api/workspaces/{ws}/tasks.
 *
 * Predefined slugs are mapped directly; a UUID with filters translates each
 * TaskViewFiltersDto field into the corresponding query parameter name.
 */
export function paramsForView(viewId: string, customFilters?: TaskViewFiltersDto): WorkspaceTaskParams {
  if (viewId === 'my-tasks') return { assignee: 'me' };
  if (viewId === 'recently-updated') return { sort: 'updated_at_desc' };
  if (viewId === 'agent-activity') return { actor: 'api_key', sort: 'updated_at_desc' };

  const filters = customFilters ?? {};
  const params: WorkspaceTaskParams = {};

  if (filters.assignee) params.assignee = filters.assignee;
  if (filters.actor_type) params.actor = filters.actor_type;
  if (filters.column_ids && filters.column_ids.length > 0) params.column_id = filters.column_ids;
  if (filters.priorities && filters.priorities.length > 0) params.priority = filters.priorities;
  if (filters.labels && filters.labels.length > 0) params.label = filters.labels;
  if (filters.board_id) params.board_id = filters.board_id;
  if (filters.sort) params.sort = filters.sort;

  return params;
}

function queryParams(params: WorkspaceTaskParams): Record<string, unknown> {
  return { ...params, limit: 200 };
}

function paramsKey(principal: string, workspaceId: string, params: WorkspaceTaskParams): string {
  const query = queryParams(params);
  const normalized = Object.fromEntries(
    Object.entries(query)
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([key, value]) => [
        key,
        Array.isArray(value) && (key === 'column_id' || key === 'label' || key === 'priority')
          ? [...value].sort()
          : value,
      ]),
  );

  return JSON.stringify({ principal, workspaceId, ...normalized });
}

/**
 * Store for flat, cross-board workspace task lists (predefined and custom views).
 *
 * M1: single-page load (limit 200). The `hasMore` flag surfaces truncation so a
 * future "Load more" affordance can be added additively without a store change.
 */
export const useWorkspaceTasksStore = defineStore('workspaceTasks', () => {
  const tasks = ref<TaskSummaryDto[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);
  const hasMore = ref(false);
  const nextCursor = ref<string | null>(null);
  const hasData = ref(false);

  let loadedKey: string | null = null;
  let activeLoadRequest: object | null = null;
  let activeTaskCacheKey: string | null = null;
  let currentQuery: {
    ws: string;
    params: WorkspaceTaskParams;
    workspaceId: string | undefined;
  } | null = null;
  const loadedPages = new Map<string, WorkspaceTaskPage>();

  function reset(): void {
    activeLoadRequest = null;
    if (activeTaskCacheKey !== null) resourceCache.deactivate(activeTaskCacheKey);
    activeTaskCacheKey = null;
    currentQuery = null;
    loadedKey = null;
    loadedPages.clear();
    tasks.value = [];
    hasMore.value = false;
    nextCursor.value = null;
    hasData.value = false;
    loading.value = false;
    error.value = null;
  }

  watch(resourceCacheEpoch, reset, { flush: 'sync' });

  function publishPage(page: WorkspaceTaskPage, key: string, request: object): void {
    if (activeLoadRequest !== request) return;

    tasks.value = page.items;
    hasMore.value = page.has_more;
    nextCursor.value = page.next_cursor;
    loadedKey = key;
    loadedPages.set(key, page);
    hasData.value = true;
    loading.value = false;
  }

  function retractDeniedPage(key: string | null, cacheKey: string): void {
    if (activeTaskCacheKey === cacheKey) activeTaskCacheKey = null;
    resourceCache.deactivate(cacheKey);
    if (key !== null) loadedPages.delete(key);
    loadedKey = null;
    tasks.value = [];
    hasMore.value = false;
    nextCursor.value = null;
    hasData.value = false;
  }

  async function load(
    ws: string,
    params: WorkspaceTaskParams,
    force = false,
    workspaceId?: string,
    options: { background?: boolean } = {},
  ): Promise<boolean> {
    currentQuery = { ws, params: { ...params }, workspaceId };

    // A background refresh (live-stream resync) keeps the mounted list visible:
    // it must not raise the loading flag nor clear `tasks`/`hasData`, and it
    // swaps the fresh page in atomically through `publishPage`. Only a list that
    // already holds data can refresh in the background; with none loaded this is
    // an initial load and stays on the destructive path (spinner + error).
    const backgroundRefresh = options.background === true && hasData.value === true;
    const principal = getResourceCachePrincipal();
    const key =
      principal === undefined || workspaceId === undefined ? null : paramsKey(principal, workspaceId, params);

    if (!force && key !== null && loadedKey === key) {
      loading.value = false;
      error.value = null;
      return true;
    }

    if (activeTaskCacheKey !== null) resourceCache.deactivate(activeTaskCacheKey);
    activeTaskCacheKey = null;

    const request = {};
    const epoch = resourceCacheEpoch.value;
    const requestParams = queryParams(params);
    const cacheKey =
      workspaceId === undefined
        ? null
        : buildCacheKey({
            principal,
            workspaceId,
            resourceKind: 'task-list',
            resourceId: 'workspace-tasks',
            query: requestParams,
            setValuedQueryKeys: ['column_id', 'label', 'priority'],
          });

    const fetchPage = async (): Promise<WorkspaceTaskPage> => {
      const { data, error: apiError } = await wrappedClient.GET('/api/workspaces/{ws}/tasks', {
        params: {
          path: { ws },
          query: requestParams,
        },
      });

      if (apiError !== undefined || data === undefined) {
        throw taskLoadError(apiError);
      }

      return { items: data.items, has_more: data.has_more, next_cursor: data.next_cursor ?? null };
    };

    const isCurrent = () => activeLoadRequest === request && resourceCacheEpoch.value === epoch;
    const cacheRequest =
      cacheKey === null
        ? null
        : {
            key: cacheKey,
            payloadSchema: workspaceTaskPageSchema(params),
            tags: ['workspace-tasks', `workspace:${workspaceId}`],
            deriveTags: (page: WorkspaceTaskPage) =>
              page.items.flatMap((task) => [`task:${task.readable_id}`, `task-uuid:${task.id}`]),
            freshForMs: CACHE_CADENCE.catalog.freshForMs,
            activeForMs: CACHE_CADENCE.catalog.activeForMs,
            retentionForMs: 24 * 60 * 60 * 1000,
            load: fetchPage,
            publish: (page: WorkspaceTaskPage) => {
              if (key !== null) publishPage(page, key, request);
            },
            isCurrent,
          };

    const remembered = !force && key !== null ? loadedPages.get(key) : undefined;
    if (remembered !== undefined) {
      activeLoadRequest = request;
      if (cacheRequest !== null && resourceCache.isAvailable()) {
        activeTaskCacheKey = cacheRequest.key;
        resourceCache.activate(cacheRequest);
      }
      tasks.value = remembered.items;
      hasMore.value = remembered.has_more;
      nextCursor.value = remembered.next_cursor;
      loadedKey = key;
      hasData.value = true;
      loading.value = false;
      error.value = null;
      return true;
    }

    activeLoadRequest = request;
    if (!backgroundRefresh) {
      loading.value = true;
      error.value = null;
      loadedKey = null;
      hasData.value = false;
      tasks.value = [];
      hasMore.value = false;
      nextCursor.value = null;
    }

    if (cacheRequest !== null && resourceCache.isAvailable()) {
      activeTaskCacheKey = cacheKey;
      try {
        await hydrateAndRevalidateResource(cacheRequest).completion;
      } catch (cause) {
        if (!isCurrent()) return false;

        const failure = taskLoadError(cause);
        const denied = failure.status === 403 || failure.status === 404;

        if (denied) {
          retractDeniedPage(key, cacheRequest.key);
          if (workspaceId !== undefined) await invalidateWorkspaceTaskQueryCache(workspaceId);
          error.value = failure.hint;
        } else if (backgroundRefresh) {
          // A transient background-refresh failure leaves the mounted list intact.
          console.warn('workspaceTasks: background refresh failed; keeping current tasks', failure);
        } else {
          error.value = failure.hint;
        }
      }

      if (!isCurrent()) return false;

      loading.value = false;
      return true;
    }

    try {
      const page = await fetchPage();

      if (!isCurrent()) return false;

      if (key !== null) publishPage(page, key, request);
      else {
        tasks.value = page.items;
        hasMore.value = page.has_more;
        nextCursor.value = page.next_cursor;
        hasData.value = true;
        loading.value = false;
      }
      return true;
    } catch (cause) {
      if (activeLoadRequest !== request) return false;

      loading.value = false;
      if (backgroundRefresh) {
        console.warn('workspaceTasks: background refresh failed; keeping current tasks', cause);
      } else {
        error.value = taskErrorHint(cause);
      }
      return true;
    }
  }

  async function invalidateCachedQueries(workspaceId: string): Promise<boolean> {
    return invalidateWorkspaceTaskQueryCache(workspaceId);
  }

  async function refreshCurrent(): Promise<boolean> {
    if (currentQuery === null) return true;
    return load(currentQuery.ws, currentQuery.params, true, currentQuery.workspaceId);
  }

  return {
    tasks,
    loading,
    error,
    hasMore,
    nextCursor,
    hasData,
    load,
    invalidateCachedQueries,
    refreshCurrent,
  };
});
