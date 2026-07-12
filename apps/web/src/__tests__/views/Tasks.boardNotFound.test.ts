import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { reactive } from 'vue';
import { useLastViewedStore } from '@/stores/lastViewed';
import { useWorkspaceStore } from '@/stores/workspace';
import Tasks from '@/views/Tasks.vue';

const { GET } = vi.hoisted(() => ({ GET: vi.fn() }));
const route = vi.hoisted(() => ({
  params: { boardId: 'board-2' } as Record<string, string>,
  query: {} as Record<string, string>,
  fullPath: '/t/board-2',
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
        ErrorState: { props: ['title', 'hint'], template: '<div data-test="error">{{ title }}</div>' },
        EmptyState: {
          props: ['title', 'hint', 'icon'],
          template: '<div data-test="empty">{{ title }}</div>',
        },
        LoadingState: { template: '<div data-test="loading">Loading</div>' },
        Icon: true,
        TaskDetailPane: true,
      },
    },
  });
}

describe('Tasks restored-board 404 fallback (ATL-77)', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    router.replace.mockResolvedValue(undefined);
    for (const key of Object.keys(route.params)) delete route.params[key];
    route.params.boardId = 'board-2';
    try {
      localStorage.clear();
    } catch {
      // jsdom provides localStorage; ignore if absent
    }
  });

  it('renders an empty state (not an error) and drops the dead entry for a deleted board', async () => {
    GET.mockResolvedValue({ data: undefined, error: { status: 404 } });

    const workspace = useWorkspaceStore();
    workspace.activeWorkspaceSlug = 'ws';
    workspace.loadMembers = vi.fn().mockResolvedValue(undefined);

    const lastViewed = useLastViewedStore();
    lastViewed.record('ws', { name: 'tasks', params: { boardId: 'board-2' } });

    const wrapper = mountTasks();
    await flushPromises();

    expect(wrapper.find('[data-test="empty"]').text()).toBe('Board not found');
    expect(wrapper.find('[data-test="error"]').exists()).toBe(false);
    expect(lastViewed.forWorkspace('ws')).toBeNull();
  });

  it('keeps the error state for a non-404 board load failure', async () => {
    GET.mockResolvedValue({ data: undefined, error: { status: 500 } });

    const workspace = useWorkspaceStore();
    workspace.activeWorkspaceSlug = 'ws';
    workspace.loadMembers = vi.fn().mockResolvedValue(undefined);

    const lastViewed = useLastViewedStore();
    lastViewed.record('ws', { name: 'tasks', params: { boardId: 'board-2' } });

    const wrapper = mountTasks();
    await flushPromises();

    expect(wrapper.find('[data-test="error"]').text()).toBe("Couldn't load board");
    expect(wrapper.find('[data-test="empty"]').exists()).toBe(false);
    expect(lastViewed.forWorkspace('ws')).toEqual({ name: 'tasks', params: { boardId: 'board-2' } });
  });
});
