interface TaskWithBoard {
  board_id?: string | null;
}

interface RouterLike {
  options: {
    history: {
      state: {
        back?: unknown;
      };
    };
  };
  back: () => void;
  push: (location: { name: 'tasks'; params?: { boardId: string } }) => Promise<unknown> | undefined;
}

interface RouteLike {
  fullPath: string;
}

interface SafeBackOrBoardOptions {
  task: TaskWithBoard;
  router: RouterLike;
  route: RouteLike;
}

export async function safeBackOrBoard({ task, router, route }: SafeBackOrBoardOptions): Promise<void> {
  const back = router.options.history.state.back;

  if (typeof back === 'string' && isUsefulSameAppBack(back, route.fullPath)) {
    router.back();
    return;
  }

  if (typeof task.board_id === 'string' && task.board_id.length > 0) {
    await router.push({ name: 'tasks', params: { boardId: task.board_id } });
    return;
  }

  await router.push({ name: 'tasks' });
}

function isUsefulSameAppBack(back: string, currentPath: string): boolean {
  if (back.length === 0 || back === currentPath) return false;
  if (back.startsWith('/')) return !back.startsWith('//');

  try {
    const url = new URL(back, window.location.origin);
    return url.origin === window.location.origin && `${url.pathname}${url.search}${url.hash}` !== currentPath;
  } catch {
    return false;
  }
}
