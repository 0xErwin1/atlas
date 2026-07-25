import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { reactive } from 'vue';
import type { BoardDto } from '@/stores/boards';
import { useBoardsStore } from '@/stores/boards';
import Tasks from '@/views/Tasks.vue';

const { GET } = vi.hoisted(() => ({ GET: vi.fn().mockResolvedValue({ data: undefined }) }));
const route = vi.hoisted(() => ({
  params: { boardId: 'board-1' } as Record<string, string>,
  query: {} as Record<string, string>,
  fullPath: '/tasks/board-1',
}));
const router = vi.hoisted(() => ({ push: vi.fn(), replace: vi.fn() }));
route.params = reactive(route.params);

vi.mock('@/api/wrapper', () => ({ wrappedClient: { GET } }));
vi.mock('vue-router', () => ({ useRoute: () => route, useRouter: () => router }));
vi.mock('@/composables/useBreakpoint', () => ({ useBreakpoint: () => ({ isMobile: false }) }));
vi.mock('@/composables/useLiveUpdates', () => ({ useLiveUpdates: vi.fn() }));
vi.mock('@/composables/useOpenTaskLive', () => ({ useOpenTaskLive: () => ({ apply: vi.fn() }) }));
vi.mock('@/composables/useBoardPresence', () => ({
  useBoardPresence: () => ({ actors: [], apply: vi.fn() }),
}));

const board: BoardDto = {
  id: 'board-1',
  name: 'Agens — Roadmap',
  workspace_id: 'ws',
  project_id: 'project-1',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
  created_by: { id: 'user-1', type: 'user', display_name: 'User' },
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
        TaskDetailPane: true,
      },
    },
  });
}

async function breadcrumbs(): Promise<string[]> {
  const wrapper = mountTasks();
  useBoardsStore().board = board;
  await flushPromises();

  return wrapper.findComponent({ name: 'EditorToolbar' }).props('breadcrumbs') as string[];
}

describe('Tasks breadcrumbs', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    GET.mockResolvedValue({ data: undefined });
  });

  // The leading crumb used to be a hardcoded "Atlas", which claimed the wrong
  // workspace in every workspace that is not named Atlas.
  it('never names a workspace it was not told about', async () => {
    expect(await breadcrumbs()).not.toContain('Atlas');
  });

  // The trailing crumb used to be a static "Board", sitting next to a view menu
  // that already names the current layout, so the two disagreed on screen.
  it('leaves the current layout to the view menu instead of restating it', async () => {
    const parts = await breadcrumbs();

    expect(parts).not.toContain('Board');
    expect(parts).toEqual(['Agens — Roadmap']);
  });
});
