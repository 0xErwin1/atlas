import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';

export type AssigneeDto = components['schemas']['AssigneeDto'];
export type ReferenceDto = components['schemas']['ReferenceDto'];
export type ChecklistItemDto = components['schemas']['ChecklistItemDto'];
export type ActivityEntryDto = components['schemas']['ActivityEntryDto'];
export type ActorDto = components['schemas']['ActorDto'];

export interface AddAssigneeInput {
  assignee_id: string;
  assignee_type: string;
}

export interface PromoteResult {
  ok: boolean;
  readableId?: string;
  hint?: string;
}

interface ApiError {
  hint?: string;
}

function hintOf(err: unknown, fallback: string): string {
  if (typeof err === 'object' && err !== null) {
    const hint = (err as ApiError).hint;
    if (typeof hint === 'string' && hint.length > 0) {
      return hint;
    }
  }
  return fallback;
}

/**
 * Task detail store (REQ-W22): owns the related collections of the open task —
 * assignees (user and agent), references, checklist, and the actor-attributed
 * activity log. Mutating operations apply optimistically and roll back on error,
 * surfacing the API hint (never a stack trace).
 */
export const useTaskDetailStore = defineStore('taskDetail', () => {
  const assignees = ref<AssigneeDto[]>([]);
  const references = ref<ReferenceDto[]>([]);
  const checklist = ref<ChecklistItemDto[]>([]);
  const activity = ref<ActivityEntryDto[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);

  async function loadAll(ws: string, readableId: string): Promise<void> {
    loading.value = true;
    error.value = null;

    const path = { ws, readable_id: readableId };

    const [a, r, c, act] = await Promise.all([
      wrappedClient.GET('/v1/workspaces/{ws}/tasks/{readable_id}/assignees', { params: { path } }),
      wrappedClient.GET('/v1/workspaces/{ws}/tasks/{readable_id}/references', { params: { path } }),
      wrappedClient.GET('/v1/workspaces/{ws}/tasks/{readable_id}/checklist', { params: { path } }),
      wrappedClient.GET('/v1/workspaces/{ws}/tasks/{readable_id}/activity', { params: { path } }),
    ]);

    loading.value = false;

    if (a.data !== undefined) {
      assignees.value = a.data;
    }
    if (r.data !== undefined) {
      references.value = r.data;
    }
    if (c.data !== undefined) {
      checklist.value = c.data;
    }
    if (act.data !== undefined) {
      activity.value = act.data.items;
    }

    const firstError = a.error ?? r.error ?? c.error ?? act.error;
    if (firstError !== undefined) {
      error.value = hintOf(firstError, 'Failed to load task detail');
    }
  }

  async function addAssignee(ws: string, readableId: string, input: AddAssigneeInput): Promise<boolean> {
    error.value = null;

    const { data, error: apiError } = await wrappedClient.POST(
      '/v1/workspaces/{ws}/tasks/{readable_id}/assignees',
      {
        params: { path: { ws, readable_id: readableId } },
        body: { assignee_id: input.assignee_id, assignee_type: input.assignee_type },
      },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = hintOf(apiError, 'Failed to add assignee');
      return false;
    }

    assignees.value = [...assignees.value, data];
    return true;
  }

  async function removeAssignee(
    ws: string,
    readableId: string,
    assigneeType: string,
    assigneeId: string,
  ): Promise<boolean> {
    error.value = null;

    const snapshot = [...assignees.value];
    assignees.value = assignees.value.filter((a) => a.assignee.id !== assigneeId);

    const assigneeRef = `${assigneeType}:${assigneeId}`;

    const { error: apiError } = await wrappedClient.DELETE(
      '/v1/workspaces/{ws}/tasks/{readable_id}/assignees/{assignee_ref}',
      { params: { path: { ws, readable_id: readableId, assignee_ref: assigneeRef } } },
    );

    if (apiError !== undefined) {
      assignees.value = snapshot;
      error.value = hintOf(apiError, 'Failed to remove assignee');
      return false;
    }

    return true;
  }

  async function toggleChecklistItem(ws: string, readableId: string, itemId: string): Promise<boolean> {
    error.value = null;

    const idx = checklist.value.findIndex((i) => i.id === itemId);
    if (idx === -1) {
      return false;
    }

    const item = checklist.value[idx];
    if (item === undefined) {
      return false;
    }

    const nextChecked = !item.checked;

    const optimistic = [...checklist.value];
    optimistic[idx] = { ...item, checked: nextChecked };
    checklist.value = optimistic;

    const { data, error: apiError } = await wrappedClient.PATCH(
      '/v1/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}',
      {
        params: { path: { ws, readable_id: readableId, item_id: itemId } },
        body: { checked: nextChecked },
      },
    );

    if (apiError !== undefined || data === undefined) {
      const rolledBack = [...checklist.value];
      rolledBack[idx] = item;
      checklist.value = rolledBack;
      error.value = hintOf(apiError, 'Failed to update checklist item');
      return false;
    }

    const reconciled = [...checklist.value];
    reconciled[idx] = data;
    checklist.value = reconciled;
    return true;
  }

  async function promoteChecklistItem(
    ws: string,
    readableId: string,
    itemId: string,
    boardId: string,
    columnId: string,
  ): Promise<PromoteResult> {
    error.value = null;

    const { data, error: apiError } = await wrappedClient.POST(
      '/v1/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}/promote',
      {
        params: { path: { ws, readable_id: readableId, item_id: itemId } },
        body: { board_id: boardId, column_id: columnId },
      },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = hintOf(apiError, 'Failed to promote checklist item');
      return { ok: false, hint: error.value };
    }

    const idx = checklist.value.findIndex((i) => i.id === itemId);
    if (idx !== -1) {
      const updated = [...checklist.value];
      updated[idx] = data.checklist_item;
      checklist.value = updated;
    }

    return { ok: true, readableId: data.task.readable_id };
  }

  async function addReference(
    ws: string,
    readableId: string,
    body: components['schemas']['CreateReferenceRequest'],
  ): Promise<boolean> {
    error.value = null;

    const { data, error: apiError } = await wrappedClient.POST(
      '/v1/workspaces/{ws}/tasks/{readable_id}/references',
      { params: { path: { ws, readable_id: readableId } }, body },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = hintOf(apiError, 'Failed to add reference');
      return false;
    }

    references.value = [...references.value, data];
    return true;
  }

  async function removeReference(ws: string, readableId: string, referenceId: string): Promise<boolean> {
    error.value = null;

    const snapshot = [...references.value];
    references.value = references.value.filter((r) => r.id !== referenceId);

    const { error: apiError } = await wrappedClient.DELETE(
      '/v1/workspaces/{ws}/tasks/{readable_id}/references/{reference_id}',
      { params: { path: { ws, readable_id: readableId, reference_id: referenceId } } },
    );

    if (apiError !== undefined) {
      references.value = snapshot;
      error.value = hintOf(apiError, 'Failed to remove reference');
      return false;
    }

    return true;
  }

  function clear(): void {
    assignees.value = [];
    references.value = [];
    checklist.value = [];
    activity.value = [];
    error.value = null;
  }

  function _setForTest(data: {
    assignees?: AssigneeDto[];
    references?: ReferenceDto[];
    checklist?: ChecklistItemDto[];
    activity?: ActivityEntryDto[];
  }): void {
    assignees.value = data.assignees ?? [];
    references.value = data.references ?? [];
    checklist.value = data.checklist ?? [];
    activity.value = data.activity ?? [];
  }

  return {
    assignees,
    references,
    checklist,
    activity,
    loading,
    error,
    loadAll,
    addAssignee,
    removeAssignee,
    toggleChecklistItem,
    promoteChecklistItem,
    addReference,
    removeReference,
    clear,
    _setForTest,
  };
});
