import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { routeRef } = vi.hoisted(() => ({
  routeRef: { params: {} as Record<string, unknown> },
}));

vi.mock('vue-router', () => ({
  useRoute: () => routeRef,
}));

import { useActiveSidebarNode } from '@/composables/useActiveSidebarNode';
import type { TaskDto } from '@/stores/tasks';
import { useTasksStore } from '@/stores/tasks';

function taskOnBoard(readableId: string, boardId: string): TaskDto {
  return { readable_id: readableId, board_id: boardId } as TaskDto;
}

describe('useActiveSidebarNode', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    routeRef.params = {};
  });

  it('derives the active document slug on the notes route', () => {
    routeRef.params = { slug: 'design-notes' };

    const { activeSlug, activeBoardId, activeViewId } = useActiveSidebarNode();

    expect(activeSlug.value).toBe('design-notes');
    expect(activeBoardId.value).toBeNull();
    expect(activeViewId.value).toBeNull();
  });

  it('derives the active board id on the board route', () => {
    routeRef.params = { boardId: 'board-1' };

    const { activeBoardId, activeSlug } = useActiveSidebarNode();

    expect(activeBoardId.value).toBe('board-1');
    expect(activeSlug.value).toBeNull();
  });

  it('derives the active view id on the task-view route', () => {
    routeRef.params = { viewId: 'my-tasks' };

    const { activeViewId } = useActiveSidebarNode();

    expect(activeViewId.value).toBe('my-tasks');
  });

  it('resolves the parent board on the task-detail route once the task is loaded', () => {
    routeRef.params = { readableId: 'ATL-42' };

    const tasks = useTasksStore();
    tasks.openTask = taskOnBoard('ATL-42', 'board-7');

    const { activeBoardId } = useActiveSidebarNode();

    expect(activeBoardId.value).toBe('board-7');
  });

  it('leaves the board unresolved on task-detail until the matching task loads', () => {
    routeRef.params = { readableId: 'ATL-42' };

    const tasks = useTasksStore();
    tasks.openTask = null;

    const { activeBoardId } = useActiveSidebarNode();

    expect(activeBoardId.value).toBeNull();
  });

  it('ignores an open task that belongs to a different route than the current one', () => {
    routeRef.params = { readableId: 'ATL-42' };

    const tasks = useTasksStore();
    tasks.openTask = taskOnBoard('ATL-99', 'board-9');

    const { activeBoardId } = useActiveSidebarNode();

    expect(activeBoardId.value).toBeNull();
  });
});
