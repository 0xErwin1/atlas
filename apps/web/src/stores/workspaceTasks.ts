import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
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

function paramsKey(ws: string, params: WorkspaceTaskParams): string {
  return JSON.stringify({ ws, ...params });
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

  let loadedKey: string | null = null;
  let activeLoadRequest: object | null = null;

  async function load(ws: string, params: WorkspaceTaskParams, force = false): Promise<boolean> {
    const key = paramsKey(ws, params);

    if (!force && loadedKey === key) {
      activeLoadRequest = null;
      loading.value = false;
      error.value = null;
      return true;
    }

    const request = {};
    activeLoadRequest = request;
    loading.value = true;
    error.value = null;

    try {
      const { data, error: apiError } = await wrappedClient.GET('/api/workspaces/{ws}/tasks', {
        params: {
          path: { ws },
          query: { ...params, limit: 200 } as Record<string, unknown>,
        },
      });

      if (activeLoadRequest !== request) return false;

      activeLoadRequest = null;
      loading.value = false;
      if (apiError !== undefined || data === undefined) {
        error.value = errorHint(apiError, 'Failed to load tasks');
        return true;
      }

      tasks.value = data.items;
      hasMore.value = data.has_more;
      nextCursor.value = data.next_cursor ?? null;
      loadedKey = key;
      return true;
    } catch (cause) {
      if (activeLoadRequest !== request) return false;

      activeLoadRequest = null;
      loading.value = false;
      error.value = errorHint(cause, 'Failed to load tasks');
      return true;
    }
  }

  return { tasks, loading, error, hasMore, nextCursor, load };
});
