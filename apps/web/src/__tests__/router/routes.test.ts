import { describe, expect, it } from 'vitest';
import { createMemoryHistory, createRouter, type RouteRecordRaw } from 'vue-router';
import { routes } from '@/router/routes';

const stub = { template: '<div />' };

// Swaps the real lazy-loaded views for a stub while preserving path, name,
// redirect and per-route guards, so navigation exercises the real routing table
// without importing heavy view components.
function stubComponents(records: RouteRecordRaw[]): RouteRecordRaw[] {
  return records.map((record) => {
    const next = { ...record } as RouteRecordRaw & { component?: unknown; children?: RouteRecordRaw[] };
    if ('component' in next && next.component !== undefined) next.component = stub;
    if (next.children !== undefined) next.children = stubComponents(next.children);
    return next;
  });
}

function makeRouter() {
  return createRouter({ history: createMemoryHistory(), routes: stubComponents(routes) });
}

describe('routes: unified notes/tasks navigation', () => {
  it('redirects the bare /t URL to the unified /n entry', async () => {
    const router = makeRouter();

    await router.push('/t');
    await router.isReady();

    expect(router.currentRoute.value.name).toBe('notes');
    expect(router.currentRoute.value.path).toBe('/n');
  });

  it('keeps the board route live at /t/:boardId', async () => {
    const router = makeRouter();

    await router.push('/t/board-1');

    expect(router.currentRoute.value.name).toBe('tasks');
    expect(router.currentRoute.value.params.boardId).toBe('board-1');
  });

  it('keeps the task-view route live at /t/views/:viewId', async () => {
    const router = makeRouter();

    await router.push('/t/views/my-tasks');

    expect(router.currentRoute.value.name).toBe('task-view');
    expect(router.currentRoute.value.params.viewId).toBe('my-tasks');
  });

  it('keeps the task-detail route live at /t/task/:readableId', async () => {
    const router = makeRouter();

    await router.push('/t/task/ATL-42');

    expect(router.currentRoute.value.name).toBe('task-detail');
    expect(router.currentRoute.value.params.readableId).toBe('ATL-42');
  });

  it('routes a boardless tasks navigation to the unified /n entry', async () => {
    const router = makeRouter();

    await router.push({ name: 'tasks' });

    expect(router.currentRoute.value.name).toBe('notes');
  });

  it('still resolves the tasks name to a board path when a board id is given', async () => {
    const router = makeRouter();

    await router.push({ name: 'tasks', params: { boardId: 'board-9' } });

    expect(router.currentRoute.value.name).toBe('tasks');
    expect(router.currentRoute.value.path).toBe('/t/board-9');
  });
});
