import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import { errorHint } from '@/lib/apiError';

export type AssigneeDto = components['schemas']['AssigneeDto'];
export type ReferenceDto = components['schemas']['ReferenceDto'];
export type ChecklistItemDto = components['schemas']['ChecklistItemDto'];
export type ActivityEntryDto = components['schemas']['ActivityEntryDto'];
export type ActorDto = components['schemas']['ActorDto'];
export type SubtaskDto = components['schemas']['TaskSummaryDto'];
export type TaskDto = components['schemas']['TaskDto'];
export type TaskAttachmentDto = components['schemas']['TaskAttachmentDto'];
export type CommentDto = components['schemas']['CommentDto'];

export interface AddAssigneeInput {
  assignee_id: string;
  assignee_type: string;
}

export interface PromoteResult {
  ok: boolean;
  readableId?: string;
  hint?: string;
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
  const subtasks = ref<SubtaskDto[]>([]);
  const activity = ref<ActivityEntryDto[]>([]);
  const attachments = ref<TaskAttachmentDto[]>([]);
  const comments = ref<CommentDto[]>([]);
  const commentsCursor = ref<string | null>(null);
  const commentsHasMore = ref(false);
  const loading = ref(false);
  const error = ref<string | null>(null);

  async function loadAll(ws: string, readableId: string): Promise<void> {
    loading.value = true;
    error.value = null;

    const path = { ws, readable_id: readableId };

    const [a, r, s, cl, act, at, cm] = await Promise.all([
      wrappedClient.GET('/v1/workspaces/{ws}/tasks/{readable_id}/assignees', { params: { path } }),
      wrappedClient.GET('/v1/workspaces/{ws}/tasks/{readable_id}/references', { params: { path } }),
      wrappedClient.GET('/v1/workspaces/{ws}/tasks/{readable_id}/subtasks', { params: { path } }),
      wrappedClient.GET('/v1/workspaces/{ws}/tasks/{readable_id}/checklist', { params: { path } }),
      wrappedClient.GET('/v1/workspaces/{ws}/tasks/{readable_id}/activity', { params: { path } }),
      wrappedClient.GET('/v1/workspaces/{ws}/tasks/{readable_id}/attachments', { params: { path } }),
      wrappedClient.GET('/v1/workspaces/{ws}/tasks/{readable_id}/comments', { params: { path } }),
    ]);

    loading.value = false;

    if (a.data !== undefined) {
      assignees.value = a.data;
    }
    if (r.data !== undefined) {
      references.value = r.data;
    }
    if (s.data !== undefined) {
      subtasks.value = s.data;
    }
    if (cl.data !== undefined) {
      checklist.value = cl.data;
    }
    if (act.data !== undefined) {
      activity.value = act.data.items;
    }
    if (at.data !== undefined) {
      attachments.value = at.data;
    }
    if (cm.data !== undefined) {
      comments.value = cm.data.items;
      commentsCursor.value = cm.data.next_cursor ?? null;
      commentsHasMore.value = cm.data.has_more;
    }

    const firstError = a.error ?? r.error ?? s.error ?? cl.error ?? act.error ?? at.error ?? cm.error;
    if (firstError !== undefined) {
      error.value = errorHint(firstError, 'Failed to load task detail');
    }
  }

  /**
   * Appends the next page of comments using the stored cursor. No-op when
   * there is no further page. Comments are returned oldest-first by the
   * server, so appending preserves conversation order.
   */
  async function loadMoreComments(ws: string, readableId: string): Promise<void> {
    error.value = null;

    if (!commentsHasMore.value || commentsCursor.value === null) {
      return;
    }

    const { data, error: apiError } = await wrappedClient.GET(
      '/v1/workspaces/{ws}/tasks/{readable_id}/comments',
      {
        params: {
          path: { ws, readable_id: readableId },
          query: { cursor: commentsCursor.value },
        },
      },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to load comments');
      return;
    }

    comments.value = [...comments.value, ...data.items];
    commentsCursor.value = data.next_cursor ?? null;
    commentsHasMore.value = data.has_more;
  }

  async function addComment(ws: string, readableId: string, body: string): Promise<boolean> {
    error.value = null;

    const { data, error: apiError } = await wrappedClient.POST(
      '/v1/workspaces/{ws}/tasks/{readable_id}/comments',
      { params: { path: { ws, readable_id: readableId } }, body: { body } },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to add comment');
      return false;
    }

    // The thread is oldest-first with a forward cursor. Appending the new
    // (newest) comment while earlier pages are still unloaded would place it
    // out of order and let a later "Load more" re-fetch it as a duplicate, so
    // only reflect it locally once the full thread is paged in. It is persisted
    // server-side either way.
    if (!commentsHasMore.value) {
      comments.value = [...comments.value, data];
    }
    return true;
  }

  async function removeComment(ws: string, readableId: string, commentId: string): Promise<boolean> {
    error.value = null;

    const snapshot = [...comments.value];
    comments.value = comments.value.filter((c) => c.id !== commentId);

    const { error: apiError } = await wrappedClient.DELETE(
      '/v1/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}',
      { params: { path: { ws, readable_id: readableId, comment_id: commentId } } },
    );

    if (apiError !== undefined) {
      comments.value = snapshot;
      error.value = errorHint(apiError, 'Failed to remove comment');
      return false;
    }

    return true;
  }

  /**
   * Edits an existing comment's body. The server authorizes this as author-only
   * (admins cannot edit others' comments), returning the updated DTO which is
   * swapped in place so the thread order is preserved.
   */
  async function editComment(
    ws: string,
    readableId: string,
    commentId: string,
    body: string,
  ): Promise<boolean> {
    error.value = null;

    const { data, error: apiError } = await wrappedClient.PATCH(
      '/v1/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}',
      {
        params: { path: { ws, readable_id: readableId, comment_id: commentId } },
        body: { body },
      },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to edit comment');
      return false;
    }

    const idx = comments.value.findIndex((c) => c.id === commentId);
    if (idx !== -1) {
      const updated = [...comments.value];
      updated[idx] = data;
      comments.value = updated;
    }

    return true;
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
      const problem = apiError as { detail?: string; status?: number } | undefined;
      error.value =
        problem?.status === 422
          ? (problem.detail ?? 'Cannot assign this user')
          : errorHint(apiError, 'Failed to add assignee');
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
      error.value = errorHint(apiError, 'Failed to remove assignee');
      return false;
    }

    return true;
  }

  /**
   * Re-fetches the task's activity feed so a change the acting user just made
   * appears immediately. The checklist endpoints emit no live event, so a
   * checklist mutation would otherwise not surface in the feed until an unrelated
   * reload; this keeps add/toggle/edit/remove/promote symmetric and live. Failures
   * are swallowed — a stale feed must never surface as a mutation error.
   */
  async function reloadActivity(ws: string, readableId: string): Promise<void> {
    try {
      const result = await wrappedClient.GET('/v1/workspaces/{ws}/tasks/{readable_id}/activity', {
        params: { path: { ws, readable_id: readableId } },
      });
      if (result?.data !== undefined) activity.value = result.data.items;
    } catch {
      // A stale activity feed must never surface as a mutation error.
    }
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
      error.value = errorHint(apiError, 'Failed to update checklist item');
      return false;
    }

    const reconciled = [...checklist.value];
    reconciled[idx] = data;
    checklist.value = reconciled;
    void reloadActivity(ws, readableId);
    return true;
  }

  async function updateChecklistItem(
    ws: string,
    readableId: string,
    itemId: string,
    title: string,
  ): Promise<boolean> {
    error.value = null;

    const trimmed = title.trim();
    if (trimmed === '') return false;

    const idx = checklist.value.findIndex((i) => i.id === itemId);
    if (idx === -1) {
      return false;
    }

    const item = checklist.value[idx];
    if (item === undefined || item.title === trimmed) {
      return false;
    }

    const optimistic = [...checklist.value];
    optimistic[idx] = { ...item, title: trimmed };
    checklist.value = optimistic;

    const { data, error: apiError } = await wrappedClient.PATCH(
      '/v1/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}',
      {
        params: { path: { ws, readable_id: readableId, item_id: itemId } },
        body: { title: trimmed },
      },
    );

    if (apiError !== undefined || data === undefined) {
      const rolledBack = [...checklist.value];
      rolledBack[idx] = item;
      checklist.value = rolledBack;
      error.value = errorHint(apiError, 'Failed to update checklist item');
      return false;
    }

    const reconciled = [...checklist.value];
    reconciled[idx] = data;
    checklist.value = reconciled;
    void reloadActivity(ws, readableId);
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
      error.value = errorHint(apiError, 'Failed to promote checklist item');
      return { ok: false, hint: error.value };
    }

    const idx = checklist.value.findIndex((i) => i.id === itemId);
    if (idx !== -1) {
      const updated = [...checklist.value];
      updated[idx] = data.checklist_item;
      checklist.value = updated;
    }

    void reloadActivity(ws, readableId);
    return { ok: true, readableId: data.task.readable_id };
  }

  async function addChecklistItem(ws: string, readableId: string, title: string): Promise<boolean> {
    error.value = null;

    const { data, error: apiError } = await wrappedClient.POST(
      '/v1/workspaces/{ws}/tasks/{readable_id}/checklist',
      { params: { path: { ws, readable_id: readableId } }, body: { title } },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to add sub-task');
      return false;
    }

    checklist.value = [...checklist.value, data];
    void reloadActivity(ws, readableId);
    return true;
  }

  async function removeChecklistItem(ws: string, readableId: string, itemId: string): Promise<boolean> {
    error.value = null;

    const { error: apiError } = await wrappedClient.DELETE(
      '/v1/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}',
      { params: { path: { ws, readable_id: readableId, item_id: itemId } } },
    );

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to delete sub-task');
      return false;
    }

    checklist.value = checklist.value.filter((i) => i.id !== itemId);
    void reloadActivity(ws, readableId);
    return true;
  }

  async function addSubtask(ws: string, readableId: string, title: string): Promise<boolean> {
    error.value = null;

    const { data, error: apiError } = await wrappedClient.POST(
      '/v1/workspaces/{ws}/tasks/{readable_id}/subtasks',
      { params: { path: { ws, readable_id: readableId } }, body: { title } },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to add sub-task');
      return false;
    }

    subtasks.value = [
      ...subtasks.value,
      {
        id: data.id,
        readable_id: data.readable_id,
        board_id: data.board_id,
        column_id: data.column_id,
        board_name: '',
        column_name: '',
        title: data.title,
        priority: data.priority,
        estimate: data.estimate,
        labels: data.labels ?? [],
        assignees: [],
        updated_at: data.updated_at,
      },
    ];
    return true;
  }

  /**
   * Moves a sub-task to another column (status), e.g. when its done checkbox is
   * toggled. Optimistically updates the local column_id and rolls back on error.
   */
  async function moveSubtaskToColumn(
    ws: string,
    subtaskReadableId: string,
    columnId: string,
  ): Promise<boolean> {
    error.value = null;

    const idx = subtasks.value.findIndex((s) => s.readable_id === subtaskReadableId);
    const previous = idx !== -1 ? subtasks.value[idx] : undefined;

    if (idx !== -1 && previous !== undefined) {
      const optimistic = [...subtasks.value];
      optimistic[idx] = { ...previous, column_id: columnId };
      subtasks.value = optimistic;
    }

    const { error: apiError } = await wrappedClient.POST('/v1/workspaces/{ws}/tasks/{readable_id}/move', {
      params: { path: { ws, readable_id: subtaskReadableId } },
      body: { column_id: columnId, before: null, after: null },
    });

    if (apiError !== undefined) {
      if (idx !== -1 && previous !== undefined) {
        const rolledBack = [...subtasks.value];
        rolledBack[idx] = previous;
        subtasks.value = rolledBack;
      }
      error.value = errorHint(apiError, 'Failed to update sub-task');
      return false;
    }

    return true;
  }

  async function promoteSubtask(ws: string, subtaskReadableId: string): Promise<boolean> {
    error.value = null;

    const { error: apiError } = await wrappedClient.POST('/v1/workspaces/{ws}/tasks/{readable_id}/promote', {
      params: { path: { ws, readable_id: subtaskReadableId } },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to promote sub-task');
      return false;
    }

    subtasks.value = subtasks.value.filter((s) => s.readable_id !== subtaskReadableId);
    return true;
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
      error.value = errorHint(apiError, 'Failed to add reference');
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
      error.value = errorHint(apiError, 'Failed to remove reference');
      return false;
    }

    return true;
  }

  async function uploadAttachment(ws: string, readableId: string, file: File): Promise<boolean> {
    error.value = null;

    const { data, error: apiError } = await wrappedClient.POST(
      '/v1/workspaces/{ws}/tasks/{readable_id}/attachments',
      {
        params: { path: { ws, readable_id: readableId } },
        // The body is multipart/form-data with the file in a part named `file`;
        // the FormData is assembled here so the browser sets the boundary header.
        body: '',
        bodySerializer: () => {
          const form = new FormData();
          form.append('file', file);
          return form;
        },
      },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to upload attachment');
      return false;
    }

    attachments.value = [...attachments.value, data];
    return true;
  }

  async function removeAttachment(ws: string, readableId: string, attachmentId: string): Promise<boolean> {
    error.value = null;

    const snapshot = [...attachments.value];
    attachments.value = attachments.value.filter((a) => a.id !== attachmentId);

    const { error: apiError } = await wrappedClient.DELETE(
      '/v1/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}',
      { params: { path: { ws, readable_id: readableId, attachment_id: attachmentId } } },
    );

    if (apiError !== undefined) {
      attachments.value = snapshot;
      error.value = errorHint(apiError, 'Failed to remove attachment');
      return false;
    }

    return true;
  }

  function clear(): void {
    assignees.value = [];
    references.value = [];
    checklist.value = [];
    subtasks.value = [];
    activity.value = [];
    attachments.value = [];
    comments.value = [];
    commentsCursor.value = null;
    commentsHasMore.value = false;
    error.value = null;
  }

  function _setForTest(data: {
    assignees?: AssigneeDto[];
    references?: ReferenceDto[];
    checklist?: ChecklistItemDto[];
    subtasks?: SubtaskDto[];
    activity?: ActivityEntryDto[];
    attachments?: TaskAttachmentDto[];
    comments?: CommentDto[];
    commentsCursor?: string | null;
    commentsHasMore?: boolean;
  }): void {
    assignees.value = data.assignees ?? [];
    references.value = data.references ?? [];
    checklist.value = data.checklist ?? [];
    subtasks.value = data.subtasks ?? [];
    activity.value = data.activity ?? [];
    attachments.value = data.attachments ?? [];
    comments.value = data.comments ?? [];
    commentsCursor.value = data.commentsCursor ?? null;
    commentsHasMore.value = data.commentsHasMore ?? false;
  }

  return {
    assignees,
    references,
    checklist,
    subtasks,
    activity,
    attachments,
    comments,
    commentsCursor,
    commentsHasMore,
    loading,
    error,
    loadAll,
    addAssignee,
    removeAssignee,
    toggleChecklistItem,
    updateChecklistItem,
    promoteChecklistItem,
    addChecklistItem,
    removeChecklistItem,
    addSubtask,
    moveSubtaskToColumn,
    promoteSubtask,
    addReference,
    removeReference,
    uploadAttachment,
    removeAttachment,
    loadMoreComments,
    addComment,
    editComment,
    removeComment,
    clear,
    _setForTest,
  };
});
