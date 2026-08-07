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
  return typeof value === 'string' && value.length > 0 && /^[\p{L}\p{N}._:-]+$/u.test(value);
}

function workspaceFallback(envelope: LiveEnvelope): CacheInvalidationScope | null {
  return isCanonicalWorkspaceId(envelope.workspace_id)
    ? { scope: 'workspace', workspaceId: envelope.workspace_id }
    : null;
}

/**
 * A resource scope with no tags: nothing cached depends on this event.
 *
 * Distinct from `null`, which the caller escalates to a whole-workspace purge —
 * the wrong answer for an event whose data was never cached in the first place.
 */
function nothingToInvalidate(envelope: LiveEnvelope): CacheInvalidationScope | null {
  return isCanonicalWorkspaceId(envelope.workspace_id)
    ? { scope: 'resource', workspaceId: envelope.workspace_id, tags: [] }
    : null;
}

/**
 * The tag every project-scoped catalog (the note tree: folders, documents and
 * boards of one project) carries, keyed by the project's canonical UUID.
 *
 * Catalogs are also tagged by slug, but a live envelope only routes by UUID, so
 * the UUID tag is what lets a document, folder or board event invalidate exactly
 * the one catalog it affects instead of purging the whole workspace.
 */
export function projectCatalogTag(projectId: string): string {
  return `project-uuid:${projectId}`;
}

function projectCatalogScope(envelope: LiveEnvelope): CacheInvalidationScope | null {
  const projectId = envelope.project_id;
  if (!isCanonicalWorkspaceId(projectId)) return workspaceFallback(envelope);

  return {
    scope: 'resource',
    workspaceId: envelope.workspace_id,
    tags: [projectCatalogTag(projectId)],
  };
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
    // A create, move or delete changes which documents the project's catalog
    // lists; only an in-place update leaves the catalog membership alone and can
    // be scoped to the single document body.
    if (envelope.event_type !== EVENT_TYPE.DOCUMENT_UPDATED) {
      const catalog = projectCatalogScope(envelope);
      const slug = eventString(envelope.data, 'slug');
      if (catalog?.tags !== undefined && validTagId(slug)) catalog.tags.push(`document:${slug}`);
      return catalog;
    }

    const slug = eventString(envelope.data, 'slug');
    const tags = validTagId(slug) ? [`document:${slug}`] : [];
    return tags.length === 0
      ? projectCatalogScope(envelope)
      : { scope: 'resource', workspaceId: envelope.workspace_id, tags };
  }

  const boardOrColumnEvents: ReadonlySet<string> = new Set([
    EVENT_TYPE.BOARD_CREATED,
    EVENT_TYPE.BOARD_DELETED,
    EVENT_TYPE.BOARD_MOVED,
    EVENT_TYPE.BOARD_UPDATED,
    EVENT_TYPE.COLUMN_CREATED,
    EVENT_TYPE.COLUMN_DELETED,
  ]);
  if (boardOrColumnEvents.has(envelope.event_type)) {
    const tags = ['task-board', 'workspace-tasks'];
    if (isCanonicalWorkspaceId(envelope.board_id)) tags.push(`board:${envelope.board_id}`);
    // A board appears in its project's note tree, so its lifecycle also ages
    // that catalog; a column only lives inside the board.
    if (envelope.event_type !== EVENT_TYPE.COLUMN_CREATED && envelope.event_type !== EVENT_TYPE.COLUMN_DELETED) {
      if (isCanonicalWorkspaceId(envelope.project_id)) tags.push(projectCatalogTag(envelope.project_id));
      else return workspaceFallback(envelope);
    }
    return { scope: 'resource', workspaceId: envelope.workspace_id, tags };
  }

  if (
    envelope.event_type === EVENT_TYPE.FOLDER_CREATED ||
    envelope.event_type === EVENT_TYPE.FOLDER_DELETED
  ) {
    return projectCatalogScope(envelope);
  }

  // The project list is fetched directly rather than through the resource cache,
  // so a new project ages nothing that is cached.
  if (envelope.event_type === EVENT_TYPE.PROJECT_CREATED) return nothingToInvalidate(envelope);

  // An event type this build does not know about: fail safe and drop the
  // workspace's cached state rather than serve something silently stale.
  return workspaceFallback(envelope);
}
