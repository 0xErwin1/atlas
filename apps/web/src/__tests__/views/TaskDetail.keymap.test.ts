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

const route = vi.hoisted(() => ({ params: { readableId: 'ATL-48' }, fullPath: '/t/ATL-48' }));
const router = vi.hoisted(() => ({
  back: vi.fn(),
  push: vi.fn(),
  options: { history: { state: { back: '/tasks/board-1' as unknown } } },
}));

vi.mock('vue-router', () => ({
  useRoute: () => route,
  useRouter: () => router,
}));

vi.mock('@/composables/useBreakpoint', () => ({ useBreakpoint: () => ({ isMobile: false }) }));
vi.mock('@/composables/useLiveUpdates', () => ({ useLiveUpdates: vi.fn() }));
vi.mock('@/composables/useOpenTaskLive', () => ({ useOpenTaskLive: () => ({ apply: vi.fn() }) }));

type TaskDto = components['schemas']['TaskDto'];

const task: TaskDto = {
  id: 'task-1',
  readable_id: 'ATL-48',
  board_id: 'board-1',
  board_name: 'Board',
  column_id: 'column-1',
  column_name: 'Todo',
  title: 'Keymap task',
  description: '',
  priority: null,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
  workspace_id: 'ws',
  project_id: 'project-1',
  created_by: { id: 'user-1', type: 'user', display_name: 'User' },
  labels: [],
};

function mountDetail() {
  return mount(TaskDetail, {
    global: {
      stubs: {
        AppShell: { template: '<main><slot name="sidebar" /><slot /></main>' },
        TasksSidebar: true,
        TaskDetailHeader: {
          emits: ['back'],
          template: '<button type="button" data-test="header-back" @click="$emit(\'back\')">Back</button>',
        },
        TaskBody: true,
        TaskInspector: true,
        ErrorState: true,
        LoadingState: true,
      },
    },
  });
}

async function seedTask(): Promise<void> {
  const workspace = useWorkspaceStore();
  workspace.activeWorkspaceSlug = 'ws';
  workspace.loadMembers = vi.fn().mockResolvedValue(undefined);

  const tasks = useTasksStore();
  tasks.openTask = task;
  tasks.loadTask = vi.fn().mockResolvedValue(undefined);

  const detail = useTaskDetailStore();
  detail.loadAll = vi.fn().mockResolvedValue(undefined);

  const boards = useBoardsStore();
  boards.loadBoard = vi.fn().mockResolvedValue(undefined);
  boards.loadColumns = vi.fn().mockResolvedValue(undefined);
}

describe('TaskDetail keymap wiring', () => {
  beforeEach(async () => {
    setActivePinia(createPinia());
    resetKeymapForTests();
    router.back.mockReset();
    router.push.mockReset();
    router.options.history.state.back = '/tasks/board-1';
    await seedTask();
  });

  afterEach(() => {
    resetKeymapForTests();
  });

  it('delegates standalone Escape to safe task back navigation', async () => {
    mountDetail();
    await flushPromises();

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
    await nextTick();

    expect(router.back).toHaveBeenCalledOnce();
    expect(router.push).not.toHaveBeenCalled();
  });

  it('falls back to the task board when standalone Escape has no useful back entry', async () => {
    router.options.history.state.back = null;
    mountDetail();
    await flushPromises();

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
    await nextTick();

    expect(router.push).toHaveBeenCalledWith({ name: 'tasks', params: { boardId: 'board-1' } });
  });

  it('delegates header back to useful history before board fallback', async () => {
    const wrapper = mountDetail();
    await flushPromises();

    await wrapper.find('[data-test="header-back"]').trigger('click');

    expect(router.back).toHaveBeenCalledOnce();
    expect(router.push).not.toHaveBeenCalled();
  });

  it('falls back to the task board when header back has no useful history entry', async () => {
    router.options.history.state.back = null;
    const wrapper = mountDetail();
    await flushPromises();

    await wrapper.find('[data-test="header-back"]').trigger('click');

    expect(router.back).not.toHaveBeenCalled();
    expect(router.push).toHaveBeenCalledWith({ name: 'tasks', params: { boardId: 'board-1' } });
  });

  it('does not navigate on Escape from guarded text or CodeMirror targets', async () => {
    mountDetail();
    await flushPromises();

    const input = document.createElement('input');
    document.body.appendChild(input);
    input.focus();
    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));

    const codeMirror = document.createElement('div');
    codeMirror.className = 'cm-content';
    document.body.appendChild(codeMirror);
    codeMirror.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }),
    );
    await nextTick();

    expect(router.back).not.toHaveBeenCalled();
    expect(router.push).not.toHaveBeenCalled();
    input.remove();
    codeMirror.remove();
  });
});
