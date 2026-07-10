import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { nextTick } from 'vue';
import type { components } from '@/api/types.d.ts';
import { resetKeymapForTests } from '@/composables/useKeymap';
import { useBoardsStore } from '@/stores/boards';
import { useTaskDetailStore } from '@/stores/taskDetail';
import { useTasksStore } from '@/stores/tasks';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';
import Tasks from '@/views/Tasks.vue';

const route = vi.hoisted(() => ({
  params: { boardId: 'board-1' },
  query: {} as Record<string, string>,
  fullPath: '/tasks/board-1',
}));
const router = vi.hoisted(() => ({ push: vi.fn(), replace: vi.fn() }));

vi.mock('vue-router', () => ({
  useRoute: () => route,
  useRouter: () => router,
}));

vi.mock('@/composables/useBreakpoint', () => ({ useBreakpoint: () => ({ isMobile: false }) }));
vi.mock('@/composables/useLiveUpdates', () => ({ useLiveUpdates: vi.fn() }));
vi.mock('@/composables/useOpenTaskLive', () => ({ useOpenTaskLive: () => ({ apply: vi.fn() }) }));
vi.mock('@/composables/useBoardPresence', () => ({
  useBoardPresence: () => ({ actors: [], apply: vi.fn() }),
}));

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

function mountTasks() {
  return mount(Tasks, {
    attachTo: document.body,
    global: {
      stubs: {
        AppShell: {
          template:
            '<main><slot name="sidebar-actions" /><slot name="sidebar" /><slot name="sidebar-footer" /><slot /></main>',
        },
        EditorToolbar: { template: '<section><slot name="lead" /><slot /></section>' },
        BoardViewMenu: true,
        Popover: {
          template:
            '<div><slot name="trigger" :open="false" :toggle="() => {}" /><slot :close="() => {}" /></div>',
        },
        PresenceAvatars: true,
        TasksSidebar: true,
        KanbanBoard: true,
        TaskListView: true,
        TaskTableView: true,
        TaskCalendarView: true,
        TaskTimelineView: true,
        TaskViewListView: true,
        TaskFilterPanel: true,
        ErrorState: true,
        EmptyState: true,
        LoadingState: true,
        Icon: true,
        TaskDetailPane: {
          props: ['task', 'ws'],
          emits: ['close'],
          template:
            '<aside data-test="task-pane"><button data-test="pane-close" @click="$emit(\'close\')" /></aside>',
        },
      },
    },
  });
}

async function seedLoadedBoard(): Promise<void> {
  const workspace = useWorkspaceStore();
  workspace.activeWorkspaceSlug = 'ws';
  workspace.loadMembers = vi.fn().mockResolvedValue(undefined);

  const boards = useBoardsStore();
  boards.board = {
    id: 'board-1',
    name: 'Board',
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    workspace_id: 'ws',
    project_id: 'project-1',
    created_by: { id: 'user-1', type: 'user', display_name: 'User' },
  };
  boards.columns = [];
  boards.loadBoardContents = vi.fn().mockResolvedValue(true);
  boards.loadTaskDetails = vi.fn().mockResolvedValue(undefined);

  useTaskDetailStore().loadAll = vi.fn().mockResolvedValue(undefined);
  useTasksStore().loadTask = vi.fn().mockImplementation(() => {
    useTasksStore().openTask = task;
    return Promise.resolve();
  });
}

describe('Tasks keymap wiring', () => {
  beforeEach(async () => {
    setActivePinia(createPinia());
    resetKeymapForTests();
    router.push.mockReset();
    router.replace.mockResolvedValue(undefined);
    route.params = { boardId: 'board-1' };
    route.query = {};
    await seedLoadedBoard();
  });

  afterEach(() => {
    resetKeymapForTests();
  });

  it('focuses board search via the board-search shortcut', async () => {
    const wrapper = mountTasks();
    await flushPromises();

    window.dispatchEvent(new KeyboardEvent('keydown', { key: '/', bubbles: true, cancelable: true }));
    await nextTick();

    expect(document.activeElement).toBe(wrapper.find('input[aria-label="Search tasks"]').element);
  });

  it('keeps Escape local to search clear and blur', async () => {
    const wrapper = mountTasks();
    await flushPromises();
    const ui = useUiStore();
    ui.setTaskFilterText('needle');
    const input = wrapper.find<HTMLInputElement>('input[aria-label="Search tasks"]');
    input.element.focus();

    await input.trigger('keydown', { key: 'Escape' });
    expect(ui.taskFilterText).toBe('');
    expect(document.activeElement).toBe(input.element);

    await input.trigger('keydown', { key: 'Escape' });
    expect(document.activeElement).not.toBe(input.element);
  });

  it('closes a board-hosted task detail with Escape outside guarded text targets', async () => {
    const wrapper = mountTasks();
    await flushPromises();
    await (wrapper.vm as unknown as { onSelect: (readableId: string) => Promise<void> }).onSelect('ATL-48');
    await flushPromises();
    expect(wrapper.find('[data-test="task-pane"]').exists()).toBe(true);

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
    await nextTick();

    expect(wrapper.find('[data-test="task-pane"]').exists()).toBe(false);
  });

  it('does not close the board-hosted task detail when Escape starts in text entry', async () => {
    const wrapper = mountTasks();
    await flushPromises();
    await (wrapper.vm as unknown as { onSelect: (readableId: string) => Promise<void> }).onSelect('ATL-48');
    await flushPromises();

    const text = document.createElement('textarea');
    document.body.appendChild(text);
    text.focus();
    text.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
    await nextTick();

    expect(wrapper.find('[data-test="task-pane"]').exists()).toBe(true);
    text.remove();
  });
});
