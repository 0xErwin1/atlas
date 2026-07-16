import { defineStore } from 'pinia';
import { ref, watch } from 'vue';
import { z } from 'zod';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import {
  getResourceCachePrincipal,
  hydrateAndRevalidateResource,
  invalidateTaskCache,
  resourceCache,
  resourceCacheEpoch,
} from '@/cache/cacheRuntime';
import { buildCacheKey, CACHE_CADENCE } from '@/cache/resourceCache';
import { errorHint } from '@/lib/apiError';

export type TaskDto = components['schemas']['TaskDto'];

interface TaskTarget {
  principal: string | undefined;
  readableId: string;
  ws: string;
  workspaceId: string | undefined;
}

type TaskLoadError = Error & { status?: number; hint: string };

function matchesTaskTarget(task: TaskDto, readableId: string, workspaceId?: string): boolean {
  return task.readable_id === readableId && (workspaceId === undefined || task.workspace_id === workspaceId);
}

function taskSchema(readableId: string, workspaceId?: string): z.ZodType<TaskDto> {
  return z.custom<TaskDto>((value): value is TaskDto => {
    if (typeof value !== 'object' || value === null) return false;

    const task = value as Record<string, unknown>;
    return (
      typeof task.id === 'string' &&
      typeof task.readable_id === 'string' &&
      typeof task.workspace_id === 'string' &&
      typeof task.board_id === 'string' &&
      typeof task.title === 'string' &&
      task.readable_id === readableId &&
      (workspaceId === undefined || task.workspace_id === workspaceId)
    );
  });
}

function taskLoadError(cause: unknown): TaskLoadError {
  const problem = cause as { status?: number } | undefined;
  const error = new Error(errorHint(cause, 'Failed to load task')) as TaskLoadError;
  error.hint = error.message;
  error.status = problem?.status;
  return error;
}

function matchesTarget(left: TaskTarget | null, right: TaskTarget): boolean {
  return (
    left !== null &&
    left.principal === right.principal &&
    left.ws === right.ws &&
    left.workspaceId === right.workspaceId &&
    left.readableId === right.readableId
  );
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
  // HTTP status of the last failed load, so callers can tell a missing task
  // (404) apart from a transient failure and render an empty state instead of an
  // error. Null when there is no error.
  const errorStatus = ref<number | null>(null);
  let loadSeq = 0;
  let activeTarget: TaskTarget | null = null;
  let activeCacheKey: string | null = null;
  let activeWorkspaceId: string | null = null;

  watch(resourceCacheEpoch, clear, { flush: 'sync' });

  async function loadTask(ws: string, readableId: string, workspaceId?: string): Promise<void> {
    const seq = ++loadSeq;
    const target = { principal: getResourceCachePrincipal(), ws, workspaceId, readableId };
    const epoch = resourceCacheEpoch.value;
    const targetChanged = !matchesTarget(activeTarget, target);

    if (activeCacheKey !== null) resourceCache.deactivate(activeCacheKey);
    activeCacheKey = null;
    activeTarget = target;
    activeWorkspaceId = workspaceId ?? null;
    loading.value = true;
    error.value = null;
    errorStatus.value = null;

    if (targetChanged) {
      openTask.value = null;
    }

    const load = async (): Promise<TaskDto> => {
      const { data, error: apiError } = await wrappedClient.GET('/api/workspaces/{ws}/tasks/{readable_id}', {
        params: { path: { ws, readable_id: readableId } },
      });

      if (apiError !== undefined || data === undefined) {
        throw taskLoadError(apiError);
      }

      if (!matchesTaskTarget(data, readableId, workspaceId)) {
        throw taskLoadError(new Error('Invalid task payload'));
      }

      return data;
    };

    const isCurrent = () =>
      seq === loadSeq && resourceCacheEpoch.value === epoch && matchesTarget(activeTarget, target);
    const publish = (task: TaskDto): void => {
      if (!isCurrent()) return;
      openTask.value = task;
      loading.value = false;
    };

    const cacheKey =
      workspaceId === undefined
        ? null
        : buildCacheKey({
            principal: getResourceCachePrincipal(),
            workspaceId,
            resourceKind: 'task-detail',
            resourceId: readableId,
          });

    if (cacheKey !== null && resourceCache.isAvailable()) {
      const request = {
        key: cacheKey,
        payloadSchema: taskSchema(readableId, workspaceId),
        tags: [`workspace:${workspaceId}`],
        deriveTags: (task: TaskDto) => [
          `task:${task.readable_id}`,
          `task-uuid:${task.id}`,
          `board:${task.board_id}`,
        ],
        freshForMs: CACHE_CADENCE.primary.freshForMs,
        activeForMs: CACHE_CADENCE.primary.activeForMs,
        retentionForMs: 24 * 60 * 60 * 1000,
        load,
        publish,
        isCurrent,
      };

      activeCacheKey = cacheKey;
      try {
        await hydrateAndRevalidateResource(request).completion;
      } catch (cause) {
        if (!isCurrent()) return;

        const failure = taskLoadError(cause);
        error.value = failure.hint;
        errorStatus.value = failure.status ?? null;
        if (failure.status === 403 || failure.status === 404) {
          openTask.value = null;
          if (workspaceId !== undefined) await invalidateTaskCache(workspaceId, readableId);
        }
      }

      if (isCurrent()) loading.value = false;
      return;
    }

    try {
      publish(await load());
    } catch (cause) {
      if (!isCurrent()) return;

      loading.value = false;
      const failure = taskLoadError(cause);
      error.value = failure.hint;
      errorStatus.value = failure.status ?? null;
    }
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

    if (activeWorkspaceId !== null) {
      await invalidateTaskCache(activeWorkspaceId, readableId, data.board_id);
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

  async function retractTask(readableId: string, taskUuid?: string): Promise<void> {
    if (openTask.value?.readable_id === readableId) openTask.value = null;
    if (activeWorkspaceId !== null) {
      if (taskUuid === undefined) await invalidateTaskCache(activeWorkspaceId, readableId);
      else await invalidateTaskCache(activeWorkspaceId, readableId, undefined, taskUuid);
    }
  }

  function clear(): void {
    openTask.value = null;
    loading.value = false;
    error.value = null;
    errorStatus.value = null;
    activeTarget = null;
    if (activeCacheKey !== null) resourceCache.deactivate(activeCacheKey);
    activeCacheKey = null;
    activeWorkspaceId = null;
  }

  return {
    openTask,
    loading,
    error,
    errorStatus,
    loadTask,
    updateDescription,
    patchOpenTask,
    retractTask,
    clear,
  };
});
