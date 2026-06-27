import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import { errorHint } from '@/lib/apiError';

export type TaskDto = components['schemas']['TaskDto'];

/**
 * Tasks store: holds the currently open task detail (REQ-W22).
 * The kanban board renders summaries from useBoardsStore; this store
 * owns the full TaskDto loaded when a user opens the detail panel.
 */
export const useTasksStore = defineStore('tasks', () => {
  const openTask = ref<TaskDto | null>(null);
  const loading = ref(false);
  const error = ref<string | null>(null);

  async function loadTask(ws: string, readableId: string): Promise<void> {
    loading.value = true;
    error.value = null;

    const { data, error: apiError } = await wrappedClient.GET('/v1/workspaces/{ws}/tasks/{readable_id}', {
      params: { path: { ws, readable_id: readableId } },
    });

    loading.value = false;

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to load task');
      return;
    }

    openTask.value = data;
  }

  async function updateDescription(ws: string, readableId: string, description: string): Promise<boolean> {
    error.value = null;

    const { data, error: apiError } = await wrappedClient.PATCH('/v1/workspaces/{ws}/tasks/{readable_id}', {
      params: { path: { ws, readable_id: readableId } },
      body: { description },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to update description');
      return false;
    }

    openTask.value = data;
    return true;
  }

  /**
   * Merge fields into the currently open task without a round-trip. Used after a
   * field edit (priority, status, title) goes through the boards store so the
   * standalone task route — which has no kanban summary to read from — stays in
   * sync. A no-op when no task is open.
   */
  function patchOpenTask(patch: Partial<TaskDto>): void {
    if (openTask.value === null) return;
    openTask.value = { ...openTask.value, ...patch };
  }

  function clear(): void {
    openTask.value = null;
    error.value = null;
  }

  return { openTask, loading, error, loadTask, updateDescription, patchOpenTask, clear };
});
