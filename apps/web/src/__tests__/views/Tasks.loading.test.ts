import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { reactive } from 'vue';
import { deferred } from '@/__tests__/deferred';
import type { BoardDto, ColumnDto, TaskSummaryDto } from '@/stores/boards';
import { useBoardsStore } from '@/stores/boards';
import { useTaskViewsStore } from '@/stores/taskViews';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';
import { useWorkspaceTasksStore } from '@/stores/workspaceTasks';
import Tasks from '@/views/Tasks.vue';

const { GET } = vi.hoisted(() => ({ GET: vi.fn() }));
const route = vi.hoisted(() => ({
  params: { boardId: 'board-2' } as Record<string, string>,
  query: {} as Record<string, string>,
  fullPath: '/tasks/board-2',
}));
const router = vi.hoisted(() => ({ push: vi.fn(), replace: vi.fn() }));
route.params = reactive(route.params);

vi.mock('@/api/wrapper', () => ({ wrappedClient: { GET } }));
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

const oldBoard: BoardDto = {
  id: 'board-1',
  name: 'Old board',
  workspace_id: 'ws',
  project_id: 'project-1',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
  created_by: { id: 'user-1', type: 'user', display_name: 'User' },
};

const newBoard: BoardDto = { ...oldBoard, id: 'board-2', name: 'New board' };
const oldColumn: ColumnDto = {
  id: 'old-column',
  board_id: 'board-1',
  name: 'Old column',
  position_key: 'a',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
};
const newColumn: ColumnDto = { ...oldColumn, id: 'new-column', board_id: 'board-2', name: 'New column' };
const oldTask: TaskSummaryDto = {
  id: 'old-task',
  readable_id: 'ATL-1',
  board_id: 'board-1',
  board_name: 'Old board',
  column_id: 'old-column',
  column_name: 'Old column',
  title: 'Old task',
  priority: null,
  subtask_count: 0,
  updated_at: '2026-01-01T00:00:00Z',
};
const newTask: TaskSummaryDto = {
  ...oldTask,
  id: 'new-task',
  readable_id: 'ATL-2',
  board_id: 'board-2',
  board_name: 'New board',
  column_id: 'new-column',
  column_name: 'New column',
  title: 'New task',
};

function mountTasks() {
  return mount(Tasks, {
    global: {
      stubs: {
        AppShell: { template: '<main><slot /></main>' },
        EditorToolbar: true,
        BoardViewMenu: true,
        Popover: true,
        PresenceAvatars: true,
        TasksSidebar: true,
        KanbanBoard: { template: '<div data-test="board-content">Board content</div>' },
        TaskListView: true,
        TaskTableView: true,
        TaskCalendarView: true,
        TaskTimelineView: true,
        TaskViewListView: true,
        TaskFilterPanel: true,
        ErrorState: true,
        EmptyState: true,
        LoadingState: { template: '<div data-test="loading">Loading</div>' },
        Icon: true,
        TaskDetailPane: true,
      },
    },
  });
}

describe('Tasks board loading', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    router.replace.mockResolvedValue(undefined);
    for (const key of Object.keys(route.params)) delete route.params[key];
    route.params.boardId = 'board-2';
  });

  it('shows only the loader until the new board, columns, and tasks finish loading', async () => {
    const boardResponse = deferred<{ data: BoardDto; error: undefined }>();
    const columnsResponse = deferred<{ data: ColumnDto[]; error: undefined }>();
    const tasksResponse = deferred<{
      data: { items: TaskSummaryDto[]; has_more: false; next_cursor: null };
      error: undefined;
    }>();
    GET.mockImplementation((path: string) => {
      if (path.endsWith('/columns')) return columnsResponse.promise;
      if (path.endsWith('/tasks')) return tasksResponse.promise;
      return boardResponse.promise;
    });

    const workspace = useWorkspaceStore();
    workspace.activeWorkspaceSlug = 'ws';
    workspace.loadMembers = vi.fn().mockResolvedValue(undefined);

    const boards = useBoardsStore();
    boards.board = oldBoard;
    boards.columns = [oldColumn];
    boards._setTasksForTest({ 'old-column': [oldTask] });

    const wrapper = mountTasks();

    expect(wrapper.find('[data-test="loading"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="board-content"]').exists()).toBe(false);
    expect(boards.columns).toEqual([]);
    expect(boards.tasksByColumn('old-column')).toEqual([]);

    boardResponse.resolve({ data: newBoard, error: undefined });
    await flushPromises();

    expect(wrapper.find('[data-test="loading"]').exists()).toBe(true);
    expect(wrapper.find('[data-test="board-content"]').exists()).toBe(false);

    columnsResponse.resolve({ data: [newColumn], error: undefined });
    await flushPromises();
    expect(wrapper.find('[data-test="loading"]').exists()).toBe(true);

    tasksResponse.resolve({
      data: { items: [newTask], has_more: false, next_cursor: null },
      error: undefined,
    });
    await flushPromises();

    expect(wrapper.find('[data-test="loading"]').exists()).toBe(false);
    expect(wrapper.find('[data-test="board-content"]').exists()).toBe(true);
    expect(boards.board?.id).toBe('board-2');
    expect(boards.columns.map((column) => column.id)).toEqual(['new-column']);
    expect(boards.tasksByColumn('new-column').map((task) => task.id)).toEqual(['new-task']);
  });

  it('invalidates a board load on view navigation and skips stale outer-await side effects', async () => {
    const membersResponse = deferred<void>();
    const workspace = useWorkspaceStore();
    workspace.activeWorkspaceSlug = 'ws';
    workspace.loadMembers = vi.fn().mockReturnValue(membersResponse.promise);

    const boards = useBoardsStore();
    boards.loadBoardContents = vi.fn().mockResolvedValue(true);
    const cancelBoardLoad = vi.spyOn(boards, 'cancelBoardLoad');
    useWorkspaceTasksStore().load = vi.fn().mockResolvedValue(true);
    const setTaskView = vi.spyOn(useUiStore(), 'setTaskView');

    mountTasks();
    await Promise.resolve();

    delete route.params.boardId;
    route.params.viewId = 'my-tasks';
    await flushPromises();

    expect(cancelBoardLoad).toHaveBeenCalledOnce();

    membersResponse.resolve();
    await flushPromises();

    expect(setTaskView).not.toHaveBeenCalled();
  });

  it('continues current board side effects after a member load failure settles', async () => {
    const workspace = useWorkspaceStore();
    workspace.activeWorkspaceSlug = 'ws';
    workspace.loadMembers = vi.fn().mockImplementation(async () => {
      workspace.error = 'Failed to load members';
    });

    const boards = useBoardsStore();
    boards.loadBoardContents = vi.fn().mockResolvedValue(true);
    const setTaskView = vi.spyOn(useUiStore(), 'setTaskView');

    mountTasks();
    await flushPromises();

    expect(workspace.error).toBe('Failed to load members');
    expect(setTaskView).toHaveBeenCalled();
  });

  it('uses operation identity to reject stale side effects after A to B to A navigation', async () => {
    route.params.boardId = 'board-a';
    const workspace = useWorkspaceStore();
    workspace.activeWorkspaceSlug = 'ws';
    workspace.loadMembers = vi.fn().mockResolvedValue(undefined);

    const boards = useBoardsStore();
    const boardLoads: { boardId: string; response: ReturnType<typeof deferred<boolean>> }[] = [];
    boards.loadBoardContents = vi.fn().mockImplementation((_ws: string, boardId: string) => {
      const response = deferred<boolean>();
      boardLoads.push({ boardId, response });
      return response.promise;
    });
    const setTaskView = vi.spyOn(useUiStore(), 'setTaskView');

    mountTasks();
    await Promise.resolve();

    route.params.boardId = 'board-b';
    await flushPromises();
    route.params.boardId = 'board-a';
    await flushPromises();

    const matchingLoads = boardLoads.filter((load) => load.boardId === 'board-a');
    const firstLoad = matchingLoads.at(0);
    const latestLoad = matchingLoads.at(-1);
    expect(firstLoad).toBeDefined();
    expect(latestLoad).toBeDefined();
    expect(firstLoad).not.toBe(latestLoad);

    setTaskView.mockClear();
    latestLoad?.response.resolve(true);
    await flushPromises();
    expect(setTaskView).toHaveBeenCalledOnce();

    firstLoad?.response.resolve(true);
    for (const load of boardLoads) load.response.resolve(false);
    await flushPromises();
    expect(setTaskView).toHaveBeenCalledOnce();
  });

  it('waits for custom saved view metadata before loading filtered tasks', async () => {
    const metadataResponse = deferred<{
      data: {
        id: string;
        name: string;
        workspace_id: string;
        filters: { priorities: string[] };
        created_at: string;
        updated_at: string;
      }[];
      error: undefined;
    }>();
    GET.mockReturnValue(metadataResponse.promise);
    delete route.params.boardId;
    route.params.viewId = 'custom-view';

    const workspace = useWorkspaceStore();
    workspace.activeWorkspaceSlug = 'ws';
    const workspaceTasks = useWorkspaceTasksStore();
    workspaceTasks.load = vi.fn().mockResolvedValue(true);

    mountTasks();
    await Promise.resolve();
    expect(workspaceTasks.load).not.toHaveBeenCalled();

    metadataResponse.resolve({
      data: [
        {
          id: 'custom-view',
          name: 'High priority',
          workspace_id: 'ws',
          filters: { priorities: ['high'] },
          created_at: '2026-01-01T00:00:00Z',
          updated_at: '2026-01-01T00:00:00Z',
        },
      ],
      error: undefined,
    });
    await flushPromises();

    expect(useTaskViewsStore().items).toHaveLength(1);
    expect(workspaceTasks.load).toHaveBeenCalledWith('ws', { priority: ['high'] });
  });
});
