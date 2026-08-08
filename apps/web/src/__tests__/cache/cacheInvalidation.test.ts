import { describe, expect, it } from 'vitest';
import { mapLiveCacheInvalidation } from '@/cache/cacheInvalidation';

const workspaceId = '019ef171-bbcf-7b90-9be6-5dbb382afd08';
const taskId = '019ef171-bbcf-7b90-9be6-5dbb382afd09';
const boardId = '019ef171-bbcf-7b90-9be6-5dbb382afd0a';
const projectId = '019ef171-bbcf-7b90-9be6-5dbb382afd0b';

function envelope(eventType: string, data: unknown, overrides: Record<string, unknown> = {}) {
  return {
    id: 'event-1',
    event_type: eventType,
    version: 1,
    source: 'test',
    workspace_id: workspaceId,
    occurred_at: '2026-01-01T00:00:00Z',
    actor: { type: 'user', id: 'user-1' },
    data,
    ...overrides,
  };
}

describe('live cache invalidation mapper', () => {
  it.each([
    'task.created',
    'task.updated',
    'task.deleted',
  ])('maps %s to the task and workspace query scopes', (eventType) => {
    expect(mapLiveCacheInvalidation(envelope(eventType, { task_id: taskId }, { board_id: boardId }))).toEqual(
      {
        scope: 'resource',
        workspaceId,
        tags: [`task-uuid:${taskId}`, `board:${boardId}`, 'workspace-tasks'],
      },
    );
  });

  it('maps a task move to exact UUID and conservative board/workspace tags', () => {
    expect(
      mapLiveCacheInvalidation(envelope('task.moved', { task_id: taskId }, { board_id: boardId })),
    ).toEqual({
      scope: 'resource',
      workspaceId,
      tags: [`task-uuid:${taskId}`, `board:${boardId}`, 'task-board', 'workspace-tasks'],
    });
  });

  it.each([
    envelope('task.updated', { task_id: 'not-a-uuid' }),
    envelope('unknown.event', { task_id: taskId }),
    envelope('task.updated', {}),
  ])('fails closed to only the canonical event workspace when identity is incomplete', (value) => {
    expect(mapLiveCacheInvalidation(value)).toEqual({ scope: 'workspace', workspaceId });
  });

  it('does not fabricate a scope for an event without a canonical workspace identity', () => {
    expect(
      mapLiveCacheInvalidation(envelope('task.updated', { task_id: taskId }, { workspace_id: 'not-a-uuid' })),
    ).toBeNull();
  });

  it('leaves presence updates cache-neutral', () => {
    expect(mapLiveCacheInvalidation(envelope('presence.updated', {}))).toBeNull();
  });

  it('scopes a document body update to that document alone', () => {
    expect(
      mapLiveCacheInvalidation(
        envelope('document.updated', { slug: 'existing-note' }, { project_id: projectId }),
      ),
    ).toEqual({ scope: 'resource', workspaceId, tags: ['document:existing-note'] });
  });

  it.each([
    ['document.created', { slug: 'new-note' }],
    ['document.moved', { slug: 'existing-note' }],
    ['document.deleted', { slug: 'existing-note' }],
  ])('%s ages its project catalog rather than the whole workspace', (eventType, data) => {
    expect(mapLiveCacheInvalidation(envelope(eventType, data, { project_id: projectId }))).toEqual({
      scope: 'resource',
      workspaceId,
      tags: [`project-uuid:${projectId}`, `document:${(data as { slug: string }).slug}`],
    });
  });

  it.each([
    'document.created',
    'document.moved',
    'document.deleted',
    'folder.created',
    'folder.deleted',
  ])('%s falls back to the workspace when it carries no project', (eventType) => {
    expect(mapLiveCacheInvalidation(envelope(eventType, {}))).toEqual({ scope: 'workspace', workspaceId });
  });

  it.each(['folder.created', 'folder.deleted'])('%s ages only its project catalog', (eventType) => {
    expect(mapLiveCacheInvalidation(envelope(eventType, {}, { project_id: projectId }))).toEqual({
      scope: 'resource',
      workspaceId,
      tags: [`project-uuid:${projectId}`],
    });
  });

  it.each([
    'column.created',
    'column.deleted',
  ])('%s invalidates board and workspace task collections', (eventType) => {
    expect(mapLiveCacheInvalidation(envelope(eventType, {}, { board_id: boardId }))).toEqual({
      scope: 'resource',
      workspaceId,
      tags: ['task-board', 'workspace-tasks', `board:${boardId}`],
    });
  });

  it.each([
    'board.created',
    'board.deleted',
    'board.moved',
    'board.updated',
  ])('%s also ages the project catalog the board is listed in', (eventType) => {
    expect(
      mapLiveCacheInvalidation(envelope(eventType, {}, { board_id: boardId, project_id: projectId })),
    ).toEqual({
      scope: 'resource',
      workspaceId,
      tags: ['task-board', 'workspace-tasks', `board:${boardId}`, `project-uuid:${projectId}`],
    });
  });

  it('falls back to the workspace for a board event without a project', () => {
    expect(mapLiveCacheInvalidation(envelope('board.created', {}, { board_id: boardId }))).toEqual({
      scope: 'workspace',
      workspaceId,
    });
  });

  // The project list is fetched directly, never through the resource cache, so a
  // new project must not cost the workspace its cached state.
  it('leaves project.created cache-neutral', () => {
    expect(mapLiveCacheInvalidation(envelope('project.created', {}))).toEqual({
      scope: 'resource',
      workspaceId,
      tags: [],
    });
  });
});
