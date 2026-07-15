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
import { useTasksStore } from '@/stores/tasks';

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

type CommentListResponse = components['schemas']['CommentListResponseDto'];

interface CommentPage {
  has_more: boolean;
  items: CommentDto[];
  next_cursor?: string | null;
}

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
  principal: string | undefined;
  ws: string;
  readableId: string;
  workspaceId: string | undefined;
}

interface DetailOperation {
  target: DetailTarget;
  generation: number;
}

function legacyCommentItems(data: CommentListResponse): CommentDto[] | null {
  const legacyItems: CommentDto[] = [];

  for (const item of data.items) {
    if ('type' in item) return null;
    legacyItems.push(item);
  }

  return legacyItems;
}

async function getLegacyTaskCommentPage(
  ws: string,
  readableId: string,
  cursor?: string,
): Promise<{ data?: CommentPage; error?: unknown }> {
  const { data, error } = await wrappedClient.GET('/api/workspaces/{ws}/tasks/{readable_id}/comments', {
    params: {
      path: { ws, readable_id: readableId },
      ...(cursor !== undefined ? { query: { cursor } } : {}),
    },
  });

  if (error !== undefined || data === undefined) return { error };

  const items = legacyCommentItems(data);
  if (items === null) return { error: new Error('Received an unsupported full comment feed') };

  return { data: { items, next_cursor: data.next_cursor, has_more: data.has_more } };
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

const listPayloadSchema = z.array(z.object({}).passthrough());
const pagePayloadSchema = z.object({
  has_more: z.boolean().optional(),
  items: listPayloadSchema,
  next_cursor: z.string().nullable().optional(),
});
const commentPagePayloadSchema: z.ZodType<CommentPage> = z.object({
  has_more: z.boolean(),
  items: z.array(
    z.custom<CommentDto>((value) => typeof value === 'object' && value !== null && !('type' in value)),
  ),
  next_cursor: z.string().nullable().optional(),
});

function collectionPayloadSchema(name: CollectionName): z.ZodType<unknown> {
  if (name === 'comments') return commentPagePayloadSchema;
  return name === 'backlinks' || name === 'activity' ? pagePayloadSchema : listPayloadSchema;
}

function detailLoadError(cause: unknown): Error & { status?: number } {
  if (cause instanceof Error && cause.name === 'TaskDetailLoadError')
    return cause as Error & { status?: number };

  const error = new Error(errorHint(cause, 'Failed to load task detail')) as Error & { status?: number };
  error.name = 'TaskDetailLoadError';
  error.status = (cause as { status?: number } | undefined)?.status;
  return error;
}

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
  return (
    left !== null &&
    left.principal === right.principal &&
    left.ws === right.ws &&
    left.workspaceId === right.workspaceId &&
    left.readableId === right.readableId
  );
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
  const activeCollectionCacheKeys = new Set<string>();
  let activeWorkspaceId: string | null = null;
  let activeTaskUuid: string | null = null;
  let deniedTarget: DetailTarget | null = null;

  watch(resourceCacheEpoch, clear, { flush: 'sync' });

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

  function deactivateCollectionCaches(): void {
    for (const key of activeCollectionCacheKeys) resourceCache.deactivate(key);
    activeCollectionCacheKeys.clear();
  }

  function isCurrent(sequence: number, target: DetailTarget): boolean {
    return loadSequence === sequence && sameTarget(activeTarget.value, target);
  }

  function beginOperation(ws: string, readableId: string): DetailOperation {
    return {
      target: {
        principal: getResourceCachePrincipal(),
        ws,
        readableId,
        workspaceId: activeWorkspaceId ?? undefined,
      },
      generation: targetGeneration,
    };
  }

  function isOperationCurrent(operation: DetailOperation): boolean {
    return (
      operation.generation === targetGeneration &&
      (deniedTarget === null || !sameTarget(deniedTarget, operation.target)) &&
      (activeTarget.value === null || sameTarget(activeTarget.value, operation.target))
    );
  }

  function publishOperationError(operation: DetailOperation, message: string): void {
    if (isOperationCurrent(operation)) error.value = message;
  }

  async function invalidateCurrentTaskCache(readableId: string, taskUuid?: string): Promise<void> {
    if (activeWorkspaceId !== null)
      await invalidateTaskCache(activeWorkspaceId, readableId, undefined, taskUuid);
  }

  async function retractDeniedDetail(
    target: DetailTarget,
    workspaceId: string | undefined,
    taskUuid: string | undefined,
  ): Promise<void> {
    loadSequence += 1;
    targetGeneration += 1;
    deniedTarget = target;
    deactivateCollectionCaches();
    clearCollections();
    loading.value = false;
    if (workspaceId !== undefined) {
      await invalidateTaskCache(workspaceId, target.readableId, undefined, taskUuid);
    }
    await useTasksStore().retractTask(target.readableId, taskUuid);
  }

  async function settleCollection<T>(
    name: CollectionName,
    request: () => Promise<{ data?: T; error?: unknown }>,
    sequence: number,
    target: DetailTarget,
    apply: (data: T) => void,
    workspaceId?: string,
    taskUuid?: string,
  ): Promise<void> {
    const publish = (data: T): void => {
      if (!isCurrent(sequence, target)) return;
      apply(data);
      collectionStatus.value = { ...collectionStatus.value, [name]: 'ready' };
      collectionLoaded.value = { ...collectionLoaded.value, [name]: true };
    };

    const resolve = async (pending: Promise<{ data?: T; error?: unknown }>): Promise<T> => {
      const { data, error: apiError } = await pending;
      if (apiError !== undefined || data === undefined) throw detailLoadError(apiError);
      return data;
    };

    const cacheKey =
      workspaceId === undefined || taskUuid === undefined
        ? null
        : buildCacheKey({
            principal: getResourceCachePrincipal(),
            workspaceId,
            resourceKind: 'task-secondary',
            resourceId: `${target.readableId}:${name}:initial`,
          });

    if (cacheKey !== null && resourceCache.isAvailable()) {
      const cacheRequest = {
        key: cacheKey,
        payloadSchema: collectionPayloadSchema(name) as z.ZodType<T>,
        tags: [
          `task:${target.readableId}`,
          `task-uuid:${taskUuid}`,
          `task-secondary:${target.readableId}:${name}`,
        ],
        freshForMs: CACHE_CADENCE.secondary.freshForMs,
        activeForMs: CACHE_CADENCE.secondary.activeForMs,
        retentionForMs: 24 * 60 * 60 * 1000,
        load: () => resolve(request()),
        publish,
        isCurrent: () => isCurrent(sequence, target),
      };

      activeCollectionCacheKeys.add(cacheKey);
      try {
        await hydrateAndRevalidateResource(cacheRequest).completion;
      } catch (cause) {
        if (!isCurrent(sequence, target)) return;

        const failure = detailLoadError(cause);
        collectionStatus.value = { ...collectionStatus.value, [name]: 'error' };
        collectionErrors.value = { ...collectionErrors.value, [name]: failure.message };
        if (failure.status === 403 || failure.status === 404) {
          await retractDeniedDetail(target, workspaceId, taskUuid);
        }
      }
      return;
    }

    const pending = request();
    try {
      const { data, error: apiError } = await pending;
      if (!isCurrent(sequence, target)) return;
      if (apiError !== undefined || data === undefined) throw detailLoadError(apiError);
      publish(data);
    } catch (cause) {
      if (!isCurrent(sequence, target)) return;

      collectionStatus.value = { ...collectionStatus.value, [name]: 'error' };
      collectionErrors.value = {
        ...collectionErrors.value,
        [name]:
          cause instanceof Error && cause.name === 'TaskDetailLoadError'
            ? cause.message
            : 'Failed to load task detail',
      };
      const failure = detailLoadError(cause);
      if (failure.status === 403 || failure.status === 404) {
        await retractDeniedDetail(target, workspaceId, taskUuid);
      }
    }
  }

  async function loadAll(
    ws: string,
    readableId: string,
    workspaceId?: string,
    taskUuid?: string,
  ): Promise<void> {
    const target = { principal: getResourceCachePrincipal(), ws, readableId, workspaceId };
    const targetChanged = !sameTarget(activeTarget.value, target);
    const sequence = ++loadSequence;

    if (targetChanged) targetGeneration += 1;
    deactivateCollectionCaches();
    deniedTarget = null;
    activeTarget.value = target;
    activeWorkspaceId = workspaceId ?? null;
    activeTaskUuid = taskUuid ?? null;
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
        () => wrappedClient.GET('/api/workspaces/{ws}/tasks/{readable_id}/assignees', { params: { path } }),
        sequence,
        target,
        (data) => {
          assignees.value = data;
        },
        workspaceId,
        taskUuid,
      ),
      settleCollection(
        'references',
        () => wrappedClient.GET('/api/workspaces/{ws}/tasks/{readable_id}/references', { params: { path } }),
        sequence,
        target,
        (data) => {
          references.value = data;
        },
        workspaceId,
        taskUuid,
      ),
      settleCollection(
        'backlinks',
        () => wrappedClient.GET('/api/workspaces/{ws}/tasks/{readable_id}/backlinks', { params: { path } }),
        sequence,
        target,
        (data) => {
          backlinks.value = data.items;
        },
        workspaceId,
        taskUuid,
      ),
      settleCollection(
        'subtasks',
        () => wrappedClient.GET('/api/workspaces/{ws}/tasks/{readable_id}/subtasks', { params: { path } }),
        sequence,
        target,
        (data) => {
          subtasks.value = data;
        },
        workspaceId,
        taskUuid,
      ),
      settleCollection(
        'checklist',
        () => wrappedClient.GET('/api/workspaces/{ws}/tasks/{readable_id}/checklist', { params: { path } }),
        sequence,
        target,
        (data) => {
          checklist.value = data;
        },
        workspaceId,
        taskUuid,
      ),
      settleCollection(
        'activity',
        () => wrappedClient.GET('/api/workspaces/{ws}/tasks/{readable_id}/activity', { params: { path } }),
        sequence,
        target,
        (data) => {
          activity.value = data.items;
        },
        workspaceId,
        taskUuid,
      ),
      settleCollection(
        'attachments',
        () => wrappedClient.GET('/api/workspaces/{ws}/tasks/{readable_id}/attachments', { params: { path } }),
        sequence,
        target,
        (data) => {
          attachments.value = data;
        },
        workspaceId,
        taskUuid,
      ),
      settleCollection(
        'comments',
        () => getLegacyTaskCommentPage(ws, readableId),
        sequence,
        target,
        (data) => {
          comments.value = data.items;
          commentsCursor.value = data.next_cursor ?? null;
          commentsHasMore.value = data.has_more;
        },
        workspaceId,
        taskUuid,
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

    const publish = (data: CommentPage): void => {
      if (!isOperationCurrent(operation)) return;

      const existing = new Set(comments.value.map((item) => item.id));
      comments.value = [...comments.value, ...data.items.filter((item) => !existing.has(item.id))];
      commentsCursor.value = data.next_cursor ?? null;
      commentsHasMore.value = data.has_more ?? false;
      collectionStatus.value = { ...collectionStatus.value, comments: 'ready' };
      collectionLoaded.value = { ...collectionLoaded.value, comments: true };
    };

    const load = async (): Promise<CommentPage> => {
      const { data, error: apiError } = await getLegacyTaskCommentPage(ws, readableId, cursor);
      if (apiError !== undefined || data === undefined) {
        throw detailLoadError(apiError);
      }

      return data;
    };

    const cacheKey =
      activeWorkspaceId === null || activeTaskUuid === null
        ? null
        : buildCacheKey({
            principal: getResourceCachePrincipal(),
            workspaceId: activeWorkspaceId,
            resourceKind: 'task-secondary',
            resourceId: `${readableId}:comments:${cursor}`,
          });

    try {
      if (cacheKey !== null && resourceCache.isAvailable()) {
        const request = {
          key: cacheKey,
          payloadSchema: commentPagePayloadSchema,
          tags: [
            `task:${readableId}`,
            `task-uuid:${activeTaskUuid}`,
            `task-secondary:${readableId}:comments`,
          ],
          freshForMs: CACHE_CADENCE.secondary.freshForMs,
          activeForMs: CACHE_CADENCE.secondary.activeForMs,
          retentionForMs: 24 * 60 * 60 * 1000,
          load,
          publish,
          isCurrent: () => isOperationCurrent(operation),
        };

        activeCollectionCacheKeys.add(cacheKey);
        await hydrateAndRevalidateResource(request).completion;
        return;
      }

      publish(await load());
    } catch (cause) {
      if (!isOperationCurrent(operation)) return;

      collectionStatus.value = { ...collectionStatus.value, comments: 'error' };
      const failure = detailLoadError(cause);
      collectionErrors.value = { ...collectionErrors.value, comments: failure.message };
      if (failure.status === 403 || failure.status === 404) {
        await retractDeniedDetail(
          operation.target,
          activeWorkspaceId ?? undefined,
          activeTaskUuid ?? undefined,
        );
      }
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
    await invalidateCurrentTaskCache(readableId);
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

    await invalidateCurrentTaskCache(readableId);
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

    await invalidateCurrentTaskCache(readableId);
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
    await invalidateCurrentTaskCache(readableId);
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

    await invalidateCurrentTaskCache(readableId);
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
    await invalidateCurrentTaskCache(readableId);
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
    await invalidateCurrentTaskCache(readableId);
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

    await invalidateCurrentTaskCache(readableId);
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
    await invalidateCurrentTaskCache(readableId);
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
    await invalidateCurrentTaskCache(readableId);
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
    await invalidateCurrentTaskCache(readableId);
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

    await Promise.all([
      invalidateCurrentTaskCache(subtaskReadableId, previous?.id),
      invalidateCurrentTaskCache(target?.readableId ?? subtaskReadableId, activeTaskUuid ?? undefined),
    ]);
    return isOperationCurrent(operation);
  }

  async function promoteSubtask(ws: string, subtaskReadableId: string): Promise<boolean> {
    const target = activeTarget.value;
    if (target?.ws !== undefined && target.ws !== ws) return false;
    const operation = beginOperation(ws, target?.readableId ?? subtaskReadableId);
    if (!isOperationCurrent(operation)) return false;
    error.value = null;
    const child = subtasks.value.find((subtask) => subtask.readable_id === subtaskReadableId);

    const { error: apiError } = await wrappedClient.POST('/api/workspaces/{ws}/tasks/{readable_id}/promote', {
      params: { path: { ws, readable_id: subtaskReadableId } },
    });

    if (apiError !== undefined) {
      publishOperationError(operation, errorHint(apiError, 'Failed to promote sub-task'));
      return false;
    }

    if (!isOperationCurrent(operation)) return false;
    subtasks.value = subtasks.value.filter((s) => s.readable_id !== subtaskReadableId);
    await Promise.all([
      invalidateCurrentTaskCache(subtaskReadableId, child?.id),
      invalidateCurrentTaskCache(target?.readableId ?? subtaskReadableId, activeTaskUuid ?? undefined),
    ]);
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
    const reference: ReferenceDto = {
      id: data.id,
      origins: ['manual'],
      wikilink_reference_id: null,
      manual_reference_id: data.id,
      manual_kind: data.kind,
      manual_created_at: data.created_at,
      manual_created_by: data.created_by,
      target_task_id: data.target_task_id,
      target_document_id: data.target_document_id,
      target_readable_id: data.target_readable_id,
      target_title: data.target_title,
      target_resolved: data.target_resolved,
    };
    references.value = [...references.value, reference];
    await invalidateCurrentTaskCache(readableId);
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

    await invalidateCurrentTaskCache(readableId);
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
    await invalidateCurrentTaskCache(readableId);
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

    await invalidateCurrentTaskCache(readableId);
    return isOperationCurrent(operation);
  }

  function clear(): void {
    loadSequence += 1;
    targetGeneration += 1;
    activeTarget.value = null;
    activeWorkspaceId = null;
    activeTaskUuid = null;
    deniedTarget = null;
    deactivateCollectionCaches();
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
