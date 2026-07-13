import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { nextTick } from 'vue';
import type { components } from '@/api/types.d.ts';
import { resetKeymapForTests } from '@/composables/useKeymap';
import { useBoardsStore } from '@/stores/boards';
import { useTaskDetailStore } from '@/stores/taskDetail';
import { useTasksStore } from '@/stores/tasks';
import { useWorkspaceStore } from '@/stores/workspace';
import TaskDetail from '@/views/TaskDetail.vue';

const { GET } = vi.hoisted(() => ({ GET: vi.fn() }));
const { useLiveUpdates } = vi.hoisted(() => ({ useLiveUpdates: vi.fn() }));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET },
}));

const { route } = await vi.hoisted(async () => {
  const { reactive } = await import('vue');
  return { route: reactive({ params: { readableId: 'ATL-1' } }) };
});
const router = vi.hoisted(() => ({ push: vi.fn(), options: { history: { state: { back: null } } } }));

vi.mock('vue-router', () => ({
  useRoute: () => route,
  useRouter: () => router,
}));
vi.mock('@/composables/useBreakpoint', () => ({ useBreakpoint: () => ({ isMobile: false }) }));
vi.mock('@/composables/useLiveUpdates', () => ({ useLiveUpdates }));

type TaskDto = components['schemas']['TaskDto'];

function task(readableId: string): TaskDto {
  return {
    id: `task-${readableId}`,
    readable_id: readableId,
    board_id: 'board-1',
    board_name: 'Board',
    column_id: 'column-1',
    column_name: 'Todo',
    title: `Task ${readableId}`,
    description: '',
    priority: null,
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    workspace_id: 'workspace-1',
    project_id: 'project-1',
    created_by: { id: 'user-1', type: 'user', display_name: 'User' },
    labels: [],
  };
}

function mountDetail() {
  return mount(TaskDetail, {
    global: {
      stubs: {
        AppShell: { template: '<main><slot name="sidebar" /><slot /></main>' },
        TasksSidebar: true,
        TaskDetailHeader: true,
        TaskBody: { props: ['task'], template: '<article>{{ task.title }}</article>' },
        TaskInspector: true,
        ErrorState: { props: ['title', 'hint'], template: '<div role="alert">{{ title }}: {{ hint }}</div>' },
        LoadingState: { props: ['label'], template: '<div role="status">{{ label }}</div>' },
      },
    },
  });
}

function capturedLiveHandlers(): {
  onEvent: (event: { type: string; data: Record<string, string>; envelope: object }) => void;
  onResync?: () => void;
} {
  const handlers = useLiveUpdates.mock.calls.at(-1)?.[1];
  if (handlers === undefined) throw new Error('Expected TaskDetail to register live update handlers');
  return handlers;
}

describe('TaskDetail resource loading', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    resetKeymapForTests();
    route.params.readableId = 'ATL-1';

    const workspace = useWorkspaceStore();
    workspace.activeWorkspaceSlug = 'ws-1';
    workspace.loadMembers = vi.fn().mockResolvedValue(undefined);

    const boards = useBoardsStore();
    boards.loadBoard = vi.fn().mockResolvedValue(undefined);
    boards.loadColumns = vi.fn().mockResolvedValue(undefined);

    const detail = useTaskDetailStore();
    detail.loadAll = vi.fn().mockResolvedValue(undefined);
  });

  afterEach(() => {
    resetKeymapForTests();
  });

  it('immediately hides the previous task and shows loading during a route transition', async () => {
    const tasks = useTasksStore();
    tasks.openTask = task('ATL-1');
    tasks.loadTask = vi.fn().mockImplementation(() => {
      tasks.openTask = null;
      return new Promise<void>(() => {});
    });

    const wrapper = mountDetail();
    await nextTick();

    expect(wrapper.find('[role="status"]').text()).toBe('Loading task…');
    expect(wrapper.find('article').exists()).toBe(false);
  });

  it('starts dependent loads only for the latest successful primary route target', async () => {
    let resolveFirst: () => void = () => {};
    const tasks = useTasksStore();
    tasks.loadTask = vi.fn().mockImplementation((_ws: string, readableId: string) => {
      if (readableId === 'ATL-1') {
        return new Promise<void>((resolve) => {
          resolveFirst = resolve;
        });
      }

      tasks.openTask = task(readableId);
      return Promise.resolve();
    });

    mountDetail();
    await nextTick();

    route.params.readableId = 'ATL-2';
    await flushPromises();

    resolveFirst();
    await flushPromises();

    const detail = useTaskDetailStore();
    expect(detail.loadAll).toHaveBeenCalledTimes(1);
    expect(detail.loadAll).toHaveBeenCalledWith('ws-1', 'ATL-2');
  });

  it('suppresses dependent loads after a rejected same-target primary refresh', async () => {
    const tasks = useTasksStore();
    GET.mockResolvedValueOnce({ data: task('ATL-1'), error: undefined });
    await tasks.loadTask('ws-1', 'ATL-1');
    GET.mockRejectedValueOnce(new Error('Network unavailable'));

    const workspace = useWorkspaceStore();
    const boards = useBoardsStore();
    const detail = useTaskDetailStore();

    const wrapper = mountDetail();
    await flushPromises();

    expect(tasks.openTask).toBeNull();
    expect(tasks.loading).toBe(false);
    expect(tasks.error).toBe('Failed to load task');
    expect(workspace.loadMembers).not.toHaveBeenCalled();
    expect(boards.loadBoard).not.toHaveBeenCalled();
    expect(detail.loadAll).not.toHaveBeenCalled();
    expect(wrapper.find('[role="alert"]').text()).toContain('Couldn’t load task: Failed to load task');
    expect(wrapper.find('article').exists()).toBe(false);
  });

  it('uses the real open-task boundary to exclude unrelated events and reloads on resync', async () => {
    const tasks = useTasksStore();
    tasks.openTask = task('ATL-1');
    tasks.loadTask = vi.fn().mockResolvedValue(undefined);

    mountDetail();
    await flushPromises();
    (tasks.loadTask as ReturnType<typeof vi.fn>).mockClear();

    const handlers = capturedLiveHandlers();
    handlers.onEvent({ type: 'task.updated', data: { task_id: 'task-OTHER' }, envelope: {} });
    expect(tasks.loadTask).not.toHaveBeenCalled();

    handlers.onEvent({ type: 'task.updated', data: { task_id: 'task-ATL-1' }, envelope: {} });
    expect(tasks.loadTask).toHaveBeenCalledOnce();
    expect(tasks.loadTask).toHaveBeenCalledWith('ws-1', 'ATL-1');

    handlers.onResync?.();
    await flushPromises();
    expect(tasks.loadTask).toHaveBeenCalledTimes(2);
  });
});
