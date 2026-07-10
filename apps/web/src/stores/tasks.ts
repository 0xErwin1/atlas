import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import { errorHint } from '@/lib/apiError';

export type TaskDto = components['schemas']['TaskDto'];

interface TaskTarget {
  readableId: string;
  ws: string;
}

function matchesTarget(left: TaskTarget | null, right: TaskTarget): boolean {
  return left?.ws === right.ws && left.readableId === right.readableId;
}

/**
 * Tasks store: holds the currently open task detail (REQ-W22).
 * The kanban board renders summaries from useBoardsStore; this store
 * owns the full TaskDto loaded when a user opens the detail panel.
 */
export const useTasksStore = defineStore('tasks', () => {
  const openTask = ref<TaskDto | null>(null);
  const loading = ref(false);
  const error = ref<string | null>(null);
  let loadSeq = 0;
  let activeTarget: TaskTarget | null = null;

  async function loadTask(ws: string, readableId: string): Promise<void> {
    const seq = ++loadSeq;
    const target = { ws, readableId };
    const targetChanged = !matchesTarget(activeTarget, target);

    activeTarget = target;
    loading.value = true;
    error.value = null;

    if (targetChanged) {
      openTask.value = null;
    }

    const { data, error: apiError } = await wrappedClient.GET('/api/workspaces/{ws}/tasks/{readable_id}', {
      params: { path: { ws, readable_id: readableId } },
    });

    if (seq !== loadSeq || !matchesTarget(activeTarget, target)) return;

    loading.value = false;

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to load task');
      return;
    }

    openTask.value = data;
  }

  async function updateDescription(ws: string, readableId: string, description: string): Promise<boolean> {
    error.value = null;

    const previous = openTask.value;
    if (previous?.readable_id === readableId) {
      openTask.value = { ...previous, description };
    }

    const { data, error: apiError } = await wrappedClient.PATCH('/api/workspaces/{ws}/tasks/{readable_id}', {
      params: { path: { ws, readable_id: readableId } },
      body: { description },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to update description');
      if (openTask.value?.readable_id === readableId && openTask.value.description === description) {
        openTask.value = previous;
      }
      return false;
    }

    if (openTask.value?.readable_id === readableId && openTask.value.description !== description) {
      return true;
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
    loading.value = false;
    error.value = null;
    activeTarget = null;
  }

  return { openTask, loading, error, loadTask, updateDescription, patchOpenTask, clear };
});
