import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import { errorHint } from '@/lib/apiError';

export type TaskViewDto = components['schemas']['TaskViewDto'];
export type TaskViewFiltersDto = components['schemas']['TaskViewFiltersDto'];

/**
 * Store for workspace custom task views. Mirrors savedSearches.ts 1:1 against
 * /api/workspaces/{ws}/task-views. Items are kept sorted alphabetically by name.
 *
 * Note: UpdateTaskViewRequest is non-partial — both name and filters are required
 * on every PATCH, so `update` always sends the full payload.
 */
export const useTaskViewsStore = defineStore('taskViews', () => {
  const items = ref<TaskViewDto[]>([]);
  const error = ref<string | null>(null);

  let loadedWs: string | null = null;
  let activeLoad: { ws: string; request: object; promise: Promise<boolean> } | null = null;

  function load(ws: string, force = false): Promise<boolean> {
    if (!force && loadedWs === ws) return Promise.resolve(true);
    if (!force && activeLoad?.ws === ws) return activeLoad.promise;

    const request = {};
    if (loadedWs !== ws) {
      loadedWs = null;
      items.value = [];
    }
    error.value = null;

    const promise = fetchTaskViews(ws, request);
    activeLoad = { ws, request, promise };
    return promise;
  }

  async function fetchTaskViews(ws: string, request: object): Promise<boolean> {
    try {
      const { data, error: apiError } = await wrappedClient.GET('/api/workspaces/{ws}/task-views', {
        params: { path: { ws } },
      });

      if (activeLoad?.request !== request) return false;

      activeLoad = null;
      if (apiError !== undefined || data === undefined || !Array.isArray(data)) {
        error.value = errorHint(apiError, 'Failed to load task views');
        return false;
      }

      items.value = [...data].sort((a, b) => a.name.localeCompare(b.name));
      loadedWs = ws;
      return true;
    } catch (cause) {
      if (activeLoad?.request !== request) return false;

      activeLoad = null;
      error.value = errorHint(cause, 'Failed to load task views');
      return false;
    }
  }

  async function create(
    ws: string,
    payload: { name: string; filters: TaskViewFiltersDto },
  ): Promise<TaskViewDto | null> {
    const { data, error: apiError } = await wrappedClient.POST('/api/workspaces/{ws}/task-views', {
      params: { path: { ws } },
      body: { name: payload.name, filters: payload.filters },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to create task view');
      return null;
    }

    items.value = [...items.value, data].sort((a, b) => a.name.localeCompare(b.name));
    return data;
  }

  async function update(
    ws: string,
    id: string,
    payload: { name: string; filters: TaskViewFiltersDto },
  ): Promise<boolean> {
    const { data, error: apiError } = await wrappedClient.PATCH('/api/workspaces/{ws}/task-views/{id}', {
      params: { path: { ws, id } },
      body: { name: payload.name, filters: payload.filters },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to update task view');
      return false;
    }

    items.value = items.value
      .map((v) => (v.id === id ? data : v))
      .sort((a, b) => a.name.localeCompare(b.name));

    return true;
  }

  async function remove(ws: string, id: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.DELETE('/api/workspaces/{ws}/task-views/{id}', {
      params: { path: { ws, id } },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to delete task view');
      return false;
    }

    items.value = items.value.filter((v) => v.id !== id);
    return true;
  }

  return { items, error, load, create, update, remove };
});
