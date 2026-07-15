import { describe, expect, it } from 'vitest';
import { mapLiveCacheInvalidation } from '@/cache/cacheInvalidation';

const workspaceId = '019ef171-bbcf-7b90-9be6-5dbb382afd08';
const taskId = '019ef171-bbcf-7b90-9be6-5dbb382afd09';
const boardId = '019ef171-bbcf-7b90-9be6-5dbb382afd0a';

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

  it.each([
    ['document.created', { slug: 'new-note' }, undefined],
    ['document.updated', { slug: 'existing-note' }, ['document:existing-note']],
    ['document.moved', { slug: 'existing-note' }, ['document:existing-note']],
    ['document.deleted', { slug: 'existing-note' }, ['document:existing-note']],
  ])('maps %s without leaving its note catalog fresh', (eventType, data, tags) => {
    const result = mapLiveCacheInvalidation(envelope(eventType, data));

    expect(result).toEqual(
      tags === undefined ? { scope: 'workspace', workspaceId } : { scope: 'resource', workspaceId, tags },
    );
  });

  it.each([
    'folder.created',
    'folder.deleted',
  ])('%s conservatively stales the current workspace', (eventType) => {
    expect(mapLiveCacheInvalidation(envelope(eventType, {}))).toEqual({ scope: 'workspace', workspaceId });
  });

  it.each([
    'board.created',
    'board.deleted',
    'column.created',
    'column.deleted',
  ])('%s invalidates board and workspace task collections', (eventType) => {
    expect(mapLiveCacheInvalidation(envelope(eventType, {}, { board_id: boardId }))).toEqual({
      scope: 'resource',
      workspaceId,
      tags: ['task-board', 'workspace-tasks', `board:${boardId}`],
    });
  });
});
