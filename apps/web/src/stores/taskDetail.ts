import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import { errorHint } from '@/lib/apiError';

export type AssigneeDto = components['schemas']['AssigneeDto'];
export type ReferenceDto = components['schemas']['UnifiedReferenceDto'];
export type TaskBacklinkDto = components['schemas']['TaskBacklinkDto'];
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

type CollectionName =
  | 'assignees'
  | 'references'
  | 'backlinks'
  | 'subtasks'
  | 'checklist'
  | 'activity'
  | 'attachments'
  | 'comments';

type CollectionStatus = 'idle' | 'pending' | 'ready' | 'error';

interface DetailTarget {
  ws: string;
  readableId: string;
}

interface DetailOperation {
  target: DetailTarget;
  generation: number;
}

const collectionNames: CollectionName[] = [
  'assignees',
  'references',
  'backlinks',
  'subtasks',
  'checklist',
  'activity',
  'attachments',
  'comments',
];

function initialCollectionStatus(status: CollectionStatus): Record<CollectionName, CollectionStatus> {
  return Object.fromEntries(collectionNames.map((name) => [name, status])) as Record<
    CollectionName,
    CollectionStatus
  >;
}

function initialCollectionErrors(): Record<CollectionName, string | null> {
  return Object.fromEntries(collectionNames.map((name) => [name, null])) as Record<
    CollectionName,
    string | null
  >;
}

function initialCollectionLoaded(): Record<CollectionName, boolean> {
  return Object.fromEntries(collectionNames.map((name) => [name, false])) as Record<CollectionName, boolean>;
}

function sameTarget(left: DetailTarget | null, right: DetailTarget): boolean {
  return left?.ws === right.ws && left.readableId === right.readableId;
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
  const backlinks = ref<TaskBacklinkDto[]>([]);
  const checklist = ref<ChecklistItemDto[]>([]);
  const subtasks = ref<SubtaskDto[]>([]);
  const activity = ref<ActivityEntryDto[]>([]);
  const attachments = ref<TaskAttachmentDto[]>([]);
  const comments = ref<CommentDto[]>([]);
  const commentsCursor = ref<string | null>(null);
  const commentsHasMore = ref(false);
  const loading = ref(false);
  const error = ref<string | null>(null);
  const collectionStatus = ref(initialCollectionStatus('idle'));
  const collectionErrors = ref(initialCollectionErrors());
  const collectionLoaded = ref(initialCollectionLoaded());
  const activeTarget = ref<DetailTarget | null>(null);
  let loadSequence = 0;
  let targetGeneration = 0;

  function clearCollections(): void {
    assignees.value = [];
    references.value = [];
    backlinks.value = [];
    checklist.value = [];
    subtasks.value = [];
    activity.value = [];
    attachments.value = [];
    comments.value = [];
    commentsCursor.value = null;
    commentsHasMore.value = false;
  }

  function startCollectionLoads(): void {
    collectionStatus.value = initialCollectionStatus('pending');
    collectionErrors.value = initialCollectionErrors();
  }

  function isCurrent(sequence: number, target: DetailTarget): boolean {
    return loadSequence === sequence && sameTarget(activeTarget.value, target);
  }

  function beginOperation(ws: string, readableId: string): DetailOperation {
    return { target: { ws, readableId }, generation: targetGeneration };
  }

  function isOperationCurrent(operation: DetailOperation): boolean {
    return (
      operation.generation === targetGeneration &&
      (activeTarget.value === null || sameTarget(activeTarget.value, operation.target))
    );
  }

  function publishOperationError(operation: DetailOperation, message: string): void {
    if (isOperationCurrent(operation)) error.value = message;
  }

  async function settleCollection<T>(
    name: CollectionName,
    request: Promise<{ data?: T; error?: unknown }>,
    sequence: number,
    target: DetailTarget,
    apply: (data: T) => void,
  ): Promise<void> {
    try {
      const { data, error: apiError } = await request;
      if (!isCurrent(sequence, target)) return;

      if (apiError !== undefined || data === undefined) {
        collectionStatus.value = { ...collectionStatus.value, [name]: 'error' };
        collectionErrors.value = {
          ...collectionErrors.value,
          [name]: errorHint(apiError, 'Failed to load task detail'),
        };
        return;
      }

      apply(data);
      collectionStatus.value = { ...collectionStatus.value, [name]: 'ready' };
      collectionLoaded.value = { ...collectionLoaded.value, [name]: true };
    } catch {
      if (!isCurrent(sequence, target)) return;

      collectionStatus.value = { ...collectionStatus.value, [name]: 'error' };
      collectionErrors.value = {
        ...collectionErrors.value,
        [name]: 'Failed to load task detail',
      };
    }
  }

  async function loadAll(ws: string, readableId: string): Promise<void> {
    const target = { ws, readableId };
    const targetChanged = !sameTarget(activeTarget.value, target);
    const sequence = ++loadSequence;

    if (targetChanged) targetGeneration += 1;
    activeTarget.value = target;
    if (targetChanged) {
      clearCollections();
      collectionLoaded.value = initialCollectionLoaded();
    }

    loading.value = true;
    error.value = null;
    startCollectionLoads();

    const path = { ws, readable_id: readableId };

    await Promise.all([
      settleCollection(
        'assignees',
        wrappedClient.GET('/api/workspaces/{ws}/tasks/{readable_id}/assignees', { params: { path } }),
        sequence,
        target,
        (data) => {
          assignees.value = data;
        },
      ),
      settleCollection(
        'references',
        wrappedClient.GET('/api/workspaces/{ws}/tasks/{readable_id}/references', { params: { path } }),
        sequence,
        target,
        (data) => {
          references.value = data;
        },
      ),
      settleCollection(
        'backlinks',
        wrappedClient.GET('/api/workspaces/{ws}/tasks/{readable_id}/backlinks', { params: { path } }),
        sequence,
        target,
        (data) => {
          backlinks.value = data.items;
        },
      ),
      settleCollection(
        'subtasks',
        wrappedClient.GET('/api/workspaces/{ws}/tasks/{readable_id}/subtasks', { params: { path } }),
        sequence,
        target,
        (data) => {
          subtasks.value = data;
        },
      ),
      settleCollection(
        'checklist',
        wrappedClient.GET('/api/workspaces/{ws}/tasks/{readable_id}/checklist', { params: { path } }),
        sequence,
        target,
        (data) => {
          checklist.value = data;
        },
      ),
      settleCollection(
        'activity',
        wrappedClient.GET('/api/workspaces/{ws}/tasks/{readable_id}/activity', { params: { path } }),
        sequence,
        target,
        (data) => {
          activity.value = data.items;
        },
      ),
      settleCollection(
        'attachments',
        wrappedClient.GET('/api/workspaces/{ws}/tasks/{readable_id}/attachments', { params: { path } }),
        sequence,
        target,
        (data) => {
          attachments.value = data;
        },
      ),
      settleCollection(
        'comments',
        wrappedClient.GET('/api/workspaces/{ws}/tasks/{readable_id}/comments', { params: { path } }),
        sequence,
        target,
        (data) => {
          comments.value = data.items;
          commentsCursor.value = data.next_cursor ?? null;
          commentsHasMore.value = data.has_more;
        },
      ),
    ]);

    if (isCurrent(sequence, target)) loading.value = false;
  }

  /**
   * Appends the next page of comments using the stored cursor. No-op when
   * there is no further page. Comments are returned oldest-first by the
   * server, so appending preserves conversation order.
   */
  async function loadMoreComments(ws: string, readableId: string): Promise<void> {
    const operation = beginOperation(ws, readableId);
    if (!isOperationCurrent(operation)) return;

    if (!commentsHasMore.value || commentsCursor.value === null) {
      return;
    }

    const cursor = commentsCursor.value;
    collectionStatus.value = { ...collectionStatus.value, comments: 'pending' };
    collectionErrors.value = { ...collectionErrors.value, comments: null };

    try {
      const { data, error: apiError } = await wrappedClient.GET(
        '/api/workspaces/{ws}/tasks/{readable_id}/comments',
        {
          params: {
            path: { ws, readable_id: readableId },
            query: { cursor },
          },
        },
      );
      if (!isOperationCurrent(operation)) return;

      if (apiError !== undefined || data === undefined) {
        collectionStatus.value = { ...collectionStatus.value, comments: 'error' };
        collectionErrors.value = {
          ...collectionErrors.value,
          comments: errorHint(apiError, 'Failed to load comments'),
        };
        return;
      }

      comments.value = [...comments.value, ...data.items];
      commentsCursor.value = data.next_cursor ?? null;
      commentsHasMore.value = data.has_more;
      collectionStatus.value = { ...collectionStatus.value, comments: 'ready' };
    } catch {
      if (!isOperationCurrent(operation)) return;

      collectionStatus.value = { ...collectionStatus.value, comments: 'error' };
      collectionErrors.value = { ...collectionErrors.value, comments: 'Failed to load comments' };
    }
  }

  async function addComment(ws: string, readableId: string, body: string): Promise<boolean> {
    const operation = beginOperation(ws, readableId);
    if (!isOperationCurrent(operation)) return false;
    error.value = null;

    const { data, error: apiError } = await wrappedClient.POST(
      '/api/workspaces/{ws}/tasks/{readable_id}/comments',
      { params: { path: { ws, readable_id: readableId } }, body: { body } },
    );

    if (apiError !== undefined || data === undefined) {
      publishOperationError(operation, errorHint(apiError, 'Failed to add comment'));
      return false;
    }

    if (!isOperationCurrent(operation)) return false;

    // The thread is oldest-first with a forward cursor. Appending the new
    // (newest) comment while earlier pages are still unloaded would place it
    // out of order and let a later "Load more" re-fetch it as a duplicate, so
    // only reflect it locally once the full thread is paged in. It is persisted
    // server-side either way.
    if (isOperationCurrent(operation) && !commentsHasMore.value) {
      comments.value = [...comments.value, data];
    }
    return isOperationCurrent(operation);
  }

  async function removeComment(ws: string, readableId: string, commentId: string): Promise<boolean> {
    const operation = beginOperation(ws, readableId);
    if (!isOperationCurrent(operation)) return false;
    error.value = null;

    const snapshot = [...comments.value];
    comments.value = comments.value.filter((c) => c.id !== commentId);

    const { error: apiError } = await wrappedClient.DELETE(
      '/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}',
      { params: { path: { ws, readable_id: readableId, comment_id: commentId } } },
    );

    if (apiError !== undefined) {
      if (!isOperationCurrent(operation)) return false;
      comments.value = snapshot;
      publishOperationError(operation, errorHint(apiError, 'Failed to remove comment'));
      return false;
    }

    return isOperationCurrent(operation);
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
    const operation = beginOperation(ws, readableId);
    if (!isOperationCurrent(operation)) return false;
    error.value = null;

    const { data, error: apiError } = await wrappedClient.PATCH(
      '/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}',
      {
        params: { path: { ws, readable_id: readableId, comment_id: commentId } },
        body: { body },
      },
    );

    if (apiError !== undefined || data === undefined) {
      publishOperationError(operation, errorHint(apiError, 'Failed to edit comment'));
      return false;
    }

    if (!isOperationCurrent(operation)) return false;

    const idx = comments.value.findIndex((c) => c.id === commentId);
    if (idx !== -1) {
      const updated = [...comments.value];
      updated[idx] = data;
      comments.value = updated;
    }

    return isOperationCurrent(operation);
  }

  async function addAssignee(ws: string, readableId: string, input: AddAssigneeInput): Promise<boolean> {
    const operation = beginOperation(ws, readableId);
    if (!isOperationCurrent(operation)) return false;
    error.value = null;

    const { data, error: apiError } = await wrappedClient.POST(
      '/api/workspaces/{ws}/tasks/{readable_id}/assignees',
      {
        params: { path: { ws, readable_id: readableId } },
        body: { assignee_id: input.assignee_id, assignee_type: input.assignee_type },
      },
    );

    if (apiError !== undefined || data === undefined) {
      const problem = apiError as { detail?: string; status?: number } | undefined;
      const message =
        problem?.status === 422
          ? (problem.detail ?? 'Cannot assign this user')
          : errorHint(apiError, 'Failed to add assignee');
      publishOperationError(operation, message);
      return false;
    }

    if (!isOperationCurrent(operation)) return false;
    assignees.value = [...assignees.value, data];
    return isOperationCurrent(operation);
  }

  async function removeAssignee(
    ws: string,
    readableId: string,
    assigneeType: string,
    assigneeId: string,
  ): Promise<boolean> {
    const operation = beginOperation(ws, readableId);
    if (!isOperationCurrent(operation)) return false;
    error.value = null;

    const snapshot = [...assignees.value];
    assignees.value = assignees.value.filter((a) => a.assignee.id !== assigneeId);

    const assigneeRef = `${assigneeType}:${assigneeId}`;

    const { error: apiError } = await wrappedClient.DELETE(
      '/api/workspaces/{ws}/tasks/{readable_id}/assignees/{assignee_ref}',
      { params: { path: { ws, readable_id: readableId, assignee_ref: assigneeRef } } },
    );

    if (apiError !== undefined) {
      if (!isOperationCurrent(operation)) return false;
      assignees.value = snapshot;
      publishOperationError(operation, errorHint(apiError, 'Failed to remove assignee'));
      return false;
    }

    return isOperationCurrent(operation);
  }

  /**
   * Re-fetches the task's activity feed so a change the acting user just made
   * appears immediately. The checklist endpoints emit no live event, so a
   * checklist mutation would otherwise not surface in the feed until an unrelated
   * reload; this keeps add/toggle/edit/remove/promote symmetric and live. Failures
   * are swallowed — a stale feed must never surface as a mutation error.
   */
  async function reloadActivity(ws: string, readableId: string): Promise<void> {
    const operation = beginOperation(ws, readableId);
    if (!isOperationCurrent(operation)) return;

    collectionStatus.value = { ...collectionStatus.value, activity: 'pending' };
    collectionErrors.value = { ...collectionErrors.value, activity: null };

    try {
      const result = await wrappedClient.GET('/api/workspaces/{ws}/tasks/{readable_id}/activity', {
        params: { path: { ws, readable_id: readableId } },
      });
      if (!isOperationCurrent(operation)) return;

      if (result.data !== undefined) {
        activity.value = result.data.items;
        collectionStatus.value = { ...collectionStatus.value, activity: 'ready' };
        collectionLoaded.value = { ...collectionLoaded.value, activity: true };
        return;
      }

      collectionStatus.value = { ...collectionStatus.value, activity: 'error' };
      collectionErrors.value = { ...collectionErrors.value, activity: 'Failed to load activity' };
    } catch {
      if (!isOperationCurrent(operation)) return;

      collectionStatus.value = { ...collectionStatus.value, activity: 'error' };
      collectionErrors.value = { ...collectionErrors.value, activity: 'Failed to load activity' };
    }
  }

  async function toggleChecklistItem(ws: string, readableId: string, itemId: string): Promise<boolean> {
    const operation = beginOperation(ws, readableId);
    if (!isOperationCurrent(operation)) return false;
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
      '/api/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}',
      {
        params: { path: { ws, readable_id: readableId, item_id: itemId } },
        body: { checked: nextChecked },
      },
    );

    if (apiError !== undefined || data === undefined) {
      if (!isOperationCurrent(operation)) return false;
      const rolledBack = [...checklist.value];
      rolledBack[idx] = item;
      checklist.value = rolledBack;
      publishOperationError(operation, errorHint(apiError, 'Failed to update checklist item'));
      return false;
    }

    if (!isOperationCurrent(operation)) return false;
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
    const operation = beginOperation(ws, readableId);
    if (!isOperationCurrent(operation)) return false;
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
      '/api/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}',
      {
        params: { path: { ws, readable_id: readableId, item_id: itemId } },
        body: { title: trimmed },
      },
    );

    if (apiError !== undefined || data === undefined) {
      if (!isOperationCurrent(operation)) return false;
      const rolledBack = [...checklist.value];
      rolledBack[idx] = item;
      checklist.value = rolledBack;
      publishOperationError(operation, errorHint(apiError, 'Failed to update checklist item'));
      return false;
    }

    if (!isOperationCurrent(operation)) return false;
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
    const operation = beginOperation(ws, readableId);
    if (!isOperationCurrent(operation)) return { ok: false };
    error.value = null;

    const { data, error: apiError } = await wrappedClient.POST(
      '/api/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}/promote',
      {
        params: { path: { ws, readable_id: readableId, item_id: itemId } },
        body: { board_id: boardId, column_id: columnId },
      },
    );

    if (apiError !== undefined || data === undefined) {
      const hint = errorHint(apiError, 'Failed to promote checklist item');
      publishOperationError(operation, hint);
      return { ok: false, hint };
    }

    if (!isOperationCurrent(operation)) return { ok: false };
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
    const operation = beginOperation(ws, readableId);
    if (!isOperationCurrent(operation)) return false;
    error.value = null;

    const { data, error: apiError } = await wrappedClient.POST(
      '/api/workspaces/{ws}/tasks/{readable_id}/checklist',
      { params: { path: { ws, readable_id: readableId } }, body: { title } },
    );

    if (apiError !== undefined || data === undefined) {
      publishOperationError(operation, errorHint(apiError, 'Failed to add sub-task'));
      return false;
    }

    if (!isOperationCurrent(operation)) return false;
    checklist.value = [...checklist.value, data];
    void reloadActivity(ws, readableId);
    return true;
  }

  async function removeChecklistItem(ws: string, readableId: string, itemId: string): Promise<boolean> {
    const operation = beginOperation(ws, readableId);
    if (!isOperationCurrent(operation)) return false;
    error.value = null;

    const { error: apiError } = await wrappedClient.DELETE(
      '/api/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}',
      { params: { path: { ws, readable_id: readableId, item_id: itemId } } },
    );

    if (apiError !== undefined) {
      publishOperationError(operation, errorHint(apiError, 'Failed to delete sub-task'));
      return false;
    }

    if (!isOperationCurrent(operation)) return false;
    checklist.value = checklist.value.filter((i) => i.id !== itemId);
    void reloadActivity(ws, readableId);
    return true;
  }

  async function addSubtask(ws: string, readableId: string, title: string): Promise<boolean> {
    const operation = beginOperation(ws, readableId);
    if (!isOperationCurrent(operation)) return false;
    error.value = null;

    const { data, error: apiError } = await wrappedClient.POST(
      '/api/workspaces/{ws}/tasks/{readable_id}/subtasks',
      { params: { path: { ws, readable_id: readableId } }, body: { title } },
    );

    if (apiError !== undefined || data === undefined) {
      publishOperationError(operation, errorHint(apiError, 'Failed to add sub-task'));
      return false;
    }

    if (!isOperationCurrent(operation)) return false;
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
        subtask_count: 0,
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
    const target = activeTarget.value;
    if (target?.ws !== undefined && target.ws !== ws) return false;
    const operation = beginOperation(ws, target?.readableId ?? subtaskReadableId);
    if (!isOperationCurrent(operation)) return false;
    error.value = null;

    const idx = subtasks.value.findIndex((s) => s.readable_id === subtaskReadableId);
    const previous = idx !== -1 ? subtasks.value[idx] : undefined;

    if (idx !== -1 && previous !== undefined) {
      const optimistic = [...subtasks.value];
      optimistic[idx] = { ...previous, column_id: columnId };
      subtasks.value = optimistic;
    }

    const { error: apiError } = await wrappedClient.POST('/api/workspaces/{ws}/tasks/{readable_id}/move', {
      params: { path: { ws, readable_id: subtaskReadableId } },
      body: { column_id: columnId, before: null, after: null },
    });

    if (apiError !== undefined) {
      if (!isOperationCurrent(operation)) return false;
      if (idx !== -1 && previous !== undefined) {
        const rolledBack = [...subtasks.value];
        rolledBack[idx] = previous;
        subtasks.value = rolledBack;
      }
      publishOperationError(operation, errorHint(apiError, 'Failed to update sub-task'));
      return false;
    }

    return isOperationCurrent(operation);
  }

  async function promoteSubtask(ws: string, subtaskReadableId: string): Promise<boolean> {
    const target = activeTarget.value;
    if (target?.ws !== undefined && target.ws !== ws) return false;
    const operation = beginOperation(ws, target?.readableId ?? subtaskReadableId);
    if (!isOperationCurrent(operation)) return false;
    error.value = null;

    const { error: apiError } = await wrappedClient.POST('/api/workspaces/{ws}/tasks/{readable_id}/promote', {
      params: { path: { ws, readable_id: subtaskReadableId } },
    });

    if (apiError !== undefined) {
      publishOperationError(operation, errorHint(apiError, 'Failed to promote sub-task'));
      return false;
    }

    if (!isOperationCurrent(operation)) return false;
    subtasks.value = subtasks.value.filter((s) => s.readable_id !== subtaskReadableId);
    return true;
  }

  async function addReference(
    ws: string,
    readableId: string,
    body: components['schemas']['CreateReferenceRequest'],
  ): Promise<boolean> {
    const operation = beginOperation(ws, readableId);
    if (!isOperationCurrent(operation)) return false;
    error.value = null;

    const { data, error: apiError } = await wrappedClient.POST(
      '/api/workspaces/{ws}/tasks/{readable_id}/references',
      { params: { path: { ws, readable_id: readableId } }, body },
    );

    if (apiError !== undefined || data === undefined) {
      publishOperationError(operation, errorHint(apiError, 'Failed to add reference'));
      return false;
    }

    if (!isOperationCurrent(operation)) return false;
    references.value = [...references.value, data];
    return true;
  }

  async function removeReference(ws: string, readableId: string, referenceId: string): Promise<boolean> {
    const operation = beginOperation(ws, readableId);
    if (!isOperationCurrent(operation)) return false;
    error.value = null;

    const snapshot = [...references.value];
    references.value = references.value.flatMap((reference) => {
      if (reference.manual_reference_id !== referenceId) return [reference];
      if (reference.wikilink_reference_id === null || reference.wikilink_reference_id === undefined) {
        return [];
      }

      return [
        {
          ...reference,
          id: reference.wikilink_reference_id,
          origins: ['wikilink'],
          manual_reference_id: null,
          manual_kind: null,
          manual_created_by: null,
          manual_created_at: null,
        },
      ];
    });

    const { error: apiError } = await wrappedClient.DELETE(
      '/api/workspaces/{ws}/tasks/{readable_id}/references/{reference_id}',
      { params: { path: { ws, readable_id: readableId, reference_id: referenceId } } },
    );

    if (apiError !== undefined) {
      if (!isOperationCurrent(operation)) return false;
      references.value = snapshot;
      publishOperationError(operation, errorHint(apiError, 'Failed to remove reference'));
      return false;
    }

    if (!isOperationCurrent(operation)) return false;

    const { data, error: reloadError } = await wrappedClient.GET(
      '/api/workspaces/{ws}/tasks/{readable_id}/references',
      { params: { path: { ws, readable_id: readableId } } },
    );

    if (!isOperationCurrent(operation)) return false;
    if (reloadError === undefined && data !== undefined) references.value = data;

    return true;
  }

  async function uploadAttachment(ws: string, readableId: string, file: File): Promise<boolean> {
    const operation = beginOperation(ws, readableId);
    if (!isOperationCurrent(operation)) return false;
    error.value = null;

    const { data, error: apiError } = await wrappedClient.POST(
      '/api/workspaces/{ws}/tasks/{readable_id}/attachments',
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
      publishOperationError(operation, errorHint(apiError, 'Failed to upload attachment'));
      return false;
    }

    if (!isOperationCurrent(operation)) return false;
    attachments.value = [...attachments.value, data];
    return true;
  }

  async function removeAttachment(ws: string, readableId: string, attachmentId: string): Promise<boolean> {
    const operation = beginOperation(ws, readableId);
    if (!isOperationCurrent(operation)) return false;
    error.value = null;

    const snapshot = [...attachments.value];
    attachments.value = attachments.value.filter((a) => a.id !== attachmentId);

    const { error: apiError } = await wrappedClient.DELETE(
      '/api/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}',
      { params: { path: { ws, readable_id: readableId, attachment_id: attachmentId } } },
    );

    if (apiError !== undefined) {
      if (!isOperationCurrent(operation)) return false;
      attachments.value = snapshot;
      publishOperationError(operation, errorHint(apiError, 'Failed to remove attachment'));
      return false;
    }

    return isOperationCurrent(operation);
  }

  function clear(): void {
    loadSequence += 1;
    targetGeneration += 1;
    activeTarget.value = null;
    clearCollections();
    collectionStatus.value = initialCollectionStatus('idle');
    collectionErrors.value = initialCollectionErrors();
    collectionLoaded.value = initialCollectionLoaded();
    loading.value = false;
    error.value = null;
  }

  function _setForTest(data: {
    assignees?: AssigneeDto[];
    references?: ReferenceDto[];
    backlinks?: TaskBacklinkDto[];
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
    backlinks.value = data.backlinks ?? [];
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
    backlinks,
    checklist,
    subtasks,
    activity,
    attachments,
    comments,
    commentsCursor,
    commentsHasMore,
    loading,
    error,
    collectionStatus,
    collectionErrors,
    collectionLoaded,
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
