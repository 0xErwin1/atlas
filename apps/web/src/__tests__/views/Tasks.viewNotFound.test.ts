import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { reactive } from 'vue';
import { useLastViewedStore } from '@/stores/lastViewed';
import { useWorkspaceStore } from '@/stores/workspace';
import Tasks from '@/views/Tasks.vue';

const { GET } = vi.hoisted(() => ({ GET: vi.fn() }));
const route = vi.hoisted(() => ({
  params: { viewId: 'view-123' } as Record<string, string>,
  query: {} as Record<string, string>,
  fullPath: '/t/views/view-123',
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
        KanbanBoard: true,
        TaskListView: true,
        TaskTableView: true,
        TaskCalendarView: true,
        TaskTimelineView: true,
        TaskViewListView: { template: '<div data-test="view-list">View list</div>' },
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

describe('Tasks restored task-view 404 fallback (ATL-77 FU-2)', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    router.replace.mockResolvedValue(undefined);
    for (const key of Object.keys(route.params)) delete route.params[key];
    route.params.viewId = 'view-123';
    try {
      localStorage.clear();
    } catch {
      // jsdom provides localStorage; ignore if absent
    }
  });

  it('renders an empty state (not an error) and drops the dead entry for a deleted view', async () => {
    // The workspace has no task views, so the restored custom view id is missing.
    GET.mockResolvedValue({ data: [], error: undefined });

    const workspace = useWorkspaceStore();
    workspace.activeWorkspaceSlug = 'ws';

    const lastViewed = useLastViewedStore();
    lastViewed.record('ws', { name: 'task-view', params: { viewId: 'view-123' } });

    const wrapper = mountTasks();
    await flushPromises();

    expect(wrapper.find('[data-test="empty"]').text()).toBe('View not found');
    expect(wrapper.find('[data-test="error"]').exists()).toBe(false);
    expect(wrapper.find('[data-test="view-list"]').exists()).toBe(false);
    expect(lastViewed.forWorkspace('ws')).toBeNull();
  });

  it('does not keep the view-not-found state lingering on the next navigation', async () => {
    // A deleted view first resolves to the "View not found" empty state.
    GET.mockResolvedValue({ data: [], error: undefined });

    const workspace = useWorkspaceStore();
    workspace.activeWorkspaceSlug = 'ws';

    const wrapper = mountTasks();
    await flushPromises();
    expect(wrapper.find('[data-test="empty"]').text()).toBe('View not found');

    // Navigating away (here the active workspace goes null, so loadView hits its
    // early-return guard) must clear the flag before the guard, not leave it true.
    workspace.activeWorkspaceSlug = null;
    await flushPromises();

    expect(wrapper.find('[data-test="empty"]').exists()).toBe(false);
    expect(wrapper.find('[data-test="view-list"]').exists()).toBe(true);
  });
});
