import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { reactive } from 'vue';
import { deferred } from '@/__tests__/deferred';
import type { BoardDto, ColumnDto, TaskSummaryDto } from '@/stores/boards';
import { useBoardsStore } from '@/stores/boards';
import { useNotesTabsStore } from '@/stores/notesTabs';
import { useTaskViewsStore } from '@/stores/taskViews';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';
import { useWorkspaceTasksStore } from '@/stores/workspaceTasks';
import Tasks from '@/views/Tasks.vue';

const { GET } = vi.hoisted(() => ({ GET: vi.fn() }));
const { useLiveUpdates } = vi.hoisted(() => ({ useLiveUpdates: vi.fn() }));
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
vi.mock('@/composables/useLiveUpdates', () => ({ useLiveUpdates }));
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
        DocsContent: { template: '<main><slot /></main>' },
        EditorToolbar: true,
        BoardViewMenu: true,
        Popover: true,
        PresenceAvatars: true,
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

function capturedLiveHandlers(): {
  onEvent: (event: { type: string; data: Record<string, string>; envelope: { board_id?: string } }) => void;
  onResync?: () => void;
} {
  const handlers = useLiveUpdates.mock.calls.at(-1)?.[1];
  if (handlers === undefined) throw new Error('Expected Tasks to register live update handlers');
  return handlers;
}

describe('Tasks board loading', () => {
  enableAutoUnmount(afterEach);

  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    localStorage.clear();
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
    expect(workspaceTasks.load).toHaveBeenCalledWith('ws', { priority: ['high'] }, false, undefined, {
      background: false,
    });
  });

  it('resyncs a saved view only after its metadata settles, without reloading the board', async () => {
    delete route.params.boardId;
    route.params.viewId = 'custom-view';

    const workspace = useWorkspaceStore();
    workspace.activeWorkspaceSlug = 'ws';

    const taskViews = useTaskViewsStore();
    taskViews.items = [
      {
        id: 'custom-view',
        name: 'High priority',
        workspace_id: 'ws',
        filters: { priorities: ['high'] },
        created_at: '2026-01-01T00:00:00Z',
        updated_at: '2026-01-01T00:00:00Z',
      },
    ];
    taskViews.load = vi.fn().mockResolvedValue(true);

    const boards = useBoardsStore();
    boards.loadBoardContents = vi.fn().mockResolvedValue(true);
    const workspaceTasks = useWorkspaceTasksStore();
    workspaceTasks.load = vi.fn().mockResolvedValue(true);

    mountTasks();
    await flushPromises();
    vi.mocked(taskViews.load).mockClear();
    vi.mocked(workspaceTasks.load).mockClear();
    vi.mocked(boards.loadBoardContents).mockClear();

    const metadataReload = deferred<boolean>();
    vi.mocked(taskViews.load).mockReturnValueOnce(metadataReload.promise);
    capturedLiveHandlers().onResync?.();
    await Promise.resolve();

    expect(taskViews.load).toHaveBeenCalledWith('ws');
    expect(workspaceTasks.load).not.toHaveBeenCalled();
    expect(boards.loadBoardContents).not.toHaveBeenCalled();

    metadataReload.resolve(true);
    await flushPromises();

    expect(workspaceTasks.load).toHaveBeenCalledOnce();
    expect(workspaceTasks.load).toHaveBeenCalledWith('ws', { priority: ['high'] }, true, undefined, {
      background: true,
    });
    expect(boards.loadBoardContents).not.toHaveBeenCalled();
  });

  it('filters board task events and reloads the active board on resync', async () => {
    const workspace = useWorkspaceStore();
    workspace.activeWorkspaceSlug = 'ws';
    workspace.loadMembers = vi.fn().mockResolvedValue(undefined);

    const boards = useBoardsStore();
    boards.loadBoardContents = vi.fn().mockResolvedValue(true);
    const upsertTask = vi.spyOn(boards, 'upsertTaskById').mockResolvedValue();

    mountTasks();
    await flushPromises();
    upsertTask.mockClear();
    (boards.loadBoardContents as ReturnType<typeof vi.fn>).mockClear();

    const handlers = capturedLiveHandlers();
    handlers.onEvent({
      type: 'task.updated',
      data: { task_id: 'task-1' },
      envelope: { board_id: 'other-board' },
    });
    handlers.onEvent({
      type: 'task.updated',
      data: { task_id: 'task-2' },
      envelope: { board_id: 'board-2' },
    });

    expect(upsertTask).toHaveBeenCalledOnce();
    expect(upsertTask).toHaveBeenCalledWith('ws', 'task-2');

    handlers.onResync?.();
    await flushPromises();
    expect(boards.loadBoardContents).toHaveBeenCalledWith('ws', 'board-2', undefined, { background: true });
  });

  it('opens and activates a board tab once the board loads', async () => {
    const workspace = useWorkspaceStore();
    workspace.activeWorkspaceSlug = 'ws';
    workspace.loadMembers = vi.fn().mockResolvedValue(undefined);

    const boards = useBoardsStore();
    boards.board = newBoard;
    boards.loadBoardContents = vi.fn().mockResolvedValue(true);

    const tabs = useNotesTabsStore();

    const wrapper = mountTasks();
    await flushPromises();

    expect(tabs.tabs('ws')).toEqual([{ kind: 'board', id: 'board-2', title: 'New board' }]);
    expect(tabs.activeFor('ws')).toEqual({ kind: 'board', id: 'board-2' });

    wrapper.unmount();
  });

  it('does not open a tab for a saved view', async () => {
    delete route.params.boardId;
    route.params.viewId = 'my-tasks';

    const workspace = useWorkspaceStore();
    workspace.activeWorkspaceSlug = 'ws';
    useWorkspaceTasksStore().load = vi.fn().mockResolvedValue(true);

    const tabs = useNotesTabsStore();

    const wrapper = mountTasks();
    await flushPromises();

    expect(tabs.tabs('ws')).toEqual([]);
    expect(tabs.activeFor('ws')).toBeNull();

    wrapper.unmount();
  });
});
