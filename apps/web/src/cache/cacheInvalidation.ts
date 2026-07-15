import { EVENT_TYPE, type LiveEnvelope, PRESENCE_UPDATED } from '@/lib/eventTypes';
import { isCanonicalWorkspaceId } from './resourceCache';

export interface CacheInvalidationScope {
  scope: 'resource' | 'workspace';
  workspaceId: string;
  tags?: string[];
}

function eventString(data: unknown, key: string): string | undefined {
  if (typeof data !== 'object' || data === null) return undefined;
  const value = (data as Record<string, unknown>)[key];
  return typeof value === 'string' ? value : undefined;
}

function validTagId(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0 && /^[A-Za-z0-9._:-]+$/.test(value);
}

function workspaceFallback(envelope: LiveEnvelope): CacheInvalidationScope | null {
  return isCanonicalWorkspaceId(envelope.workspace_id)
    ? { scope: 'workspace', workspaceId: envelope.workspace_id }
    : null;
}

/** Maps one SSE envelope to its smallest safe cache invalidation scope. */
export function mapLiveCacheInvalidation(envelope: LiveEnvelope): CacheInvalidationScope | null {
  if (envelope.event_type === PRESENCE_UPDATED) return null;
  if (!isCanonicalWorkspaceId(envelope.workspace_id)) return null;

  const taskEvents: ReadonlySet<string> = new Set([
    EVENT_TYPE.TASK_CREATED,
    EVENT_TYPE.TASK_UPDATED,
    EVENT_TYPE.TASK_MOVED,
    EVENT_TYPE.TASK_DELETED,
  ]);
  if (taskEvents.has(envelope.event_type)) {
    const taskId = eventString(envelope.data, 'task_id');
    if (!isCanonicalWorkspaceId(taskId)) return workspaceFallback(envelope);

    const tags = [`task-uuid:${taskId}`];
    if (isCanonicalWorkspaceId(envelope.board_id)) tags.push(`board:${envelope.board_id}`);
    if (envelope.event_type === EVENT_TYPE.TASK_MOVED) tags.push('task-board');
    tags.push('workspace-tasks');

    return { scope: 'resource', workspaceId: envelope.workspace_id, tags };
  }

  const documentEvents: ReadonlySet<string> = new Set([
    EVENT_TYPE.DOCUMENT_CREATED,
    EVENT_TYPE.DOCUMENT_UPDATED,
    EVENT_TYPE.DOCUMENT_MOVED,
    EVENT_TYPE.DOCUMENT_DELETED,
  ]);
  if (documentEvents.has(envelope.event_type)) {
    if (envelope.event_type === EVENT_TYPE.DOCUMENT_CREATED) return workspaceFallback(envelope);

    const slug = eventString(envelope.data, 'slug');
    const tags = validTagId(slug) ? [`document:${slug}`] : [];
    return tags.length === 0
      ? workspaceFallback(envelope)
      : { scope: 'resource', workspaceId: envelope.workspace_id, tags };
  }

  const boardOrColumnEvents: ReadonlySet<string> = new Set([
    EVENT_TYPE.BOARD_CREATED,
    EVENT_TYPE.BOARD_DELETED,
    EVENT_TYPE.COLUMN_CREATED,
    EVENT_TYPE.COLUMN_DELETED,
  ]);
  if (boardOrColumnEvents.has(envelope.event_type)) {
    const tags = ['task-board', 'workspace-tasks'];
    if (isCanonicalWorkspaceId(envelope.board_id)) tags.push(`board:${envelope.board_id}`);
    return { scope: 'resource', workspaceId: envelope.workspace_id, tags };
  }

  if (
    envelope.event_type === EVENT_TYPE.FOLDER_CREATED ||
    envelope.event_type === EVENT_TYPE.FOLDER_DELETED
  ) {
    return workspaceFallback(envelope);
  }

  return workspaceFallback(envelope);
}
