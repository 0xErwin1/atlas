import { describe, expect, it, vi } from 'vitest';
import { safeBackOrBoard } from '@/composables/useTaskEscapeNavigation';

function routerWithBack(back: string | null) {
  return {
    options: { history: { state: { back } } },
    back: vi.fn(),
    push: vi.fn(),
  };
}

describe('safeBackOrBoard', () => {
  it('goes back when history has a useful same-app origin', async () => {
    const router = routerWithBack('/tasks?board=b1');

    await safeBackOrBoard({ task: { board_id: 'b1' }, router, route: { fullPath: '/tasks/ATL-1' } });

    expect(router.back).toHaveBeenCalledTimes(1);
    expect(router.push).not.toHaveBeenCalled();
  });

  it('does not go back to the same path', async () => {
    const router = routerWithBack('/tasks/ATL-1');

    await safeBackOrBoard({ task: { board_id: 'b1' }, router, route: { fullPath: '/tasks/ATL-1' } });

    expect(router.back).not.toHaveBeenCalled();
    expect(router.push).toHaveBeenCalledWith({ name: 'tasks', params: { boardId: 'b1' } });
  });

  it('falls back to the task board when no useful origin exists', async () => {
    const router = routerWithBack(null);

    await safeBackOrBoard({ task: { board_id: 'board-42' }, router, route: { fullPath: '/tasks/ATL-42' } });

    expect(router.back).not.toHaveBeenCalled();
    expect(router.push).toHaveBeenCalledWith({ name: 'tasks', params: { boardId: 'board-42' } });
  });

  it('rejects protocol-relative external origins', async () => {
    const router = routerWithBack('//example.com/outside');

    await safeBackOrBoard({ task: { board_id: 'b1' }, router, route: { fullPath: '/tasks/ATL-1' } });

    expect(router.back).not.toHaveBeenCalled();
    expect(router.push).toHaveBeenCalledWith({ name: 'tasks', params: { boardId: 'b1' } });
  });

  it('falls back to the tasks route when the task has no board', async () => {
    const router = routerWithBack('https://example.com/outside');

    await safeBackOrBoard({ task: {}, router, route: { fullPath: '/tasks/ATL-42' } });

    expect(router.back).not.toHaveBeenCalled();
    expect(router.push).toHaveBeenCalledWith({ name: 'tasks' });
  });
});
