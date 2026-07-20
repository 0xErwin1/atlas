import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { defineComponent, h } from 'vue';
import { createMemoryHistory, createRouter, RouterView } from 'vue-router';

const { GET, PATCH, POST, DELETE } = vi.hoisted(() => ({
  GET: vi.fn(),
  PATCH: vi.fn(),
  POST: vi.fn(),
  DELETE: vi.fn(),
}));

vi.mock('@/api/wrapper', () => ({ wrappedClient: { GET, PATCH, POST, DELETE } }));

const { useLiveUpdates } = vi.hoisted(() => ({ useLiveUpdates: vi.fn() }));
vi.mock('@/composables/useLiveUpdates', () => ({ useLiveUpdates }));
vi.mock('@/composables/useBreakpoint', () => ({ useBreakpoint: () => ({ isMobile: false }) }));

import { useNotesTabsStore } from '@/stores/notesTabs';
import { useWorkspaceStore } from '@/stores/workspace';
import NotesSidebar from '@/views/NotesSidebar.vue';
import WorkspaceShell from '@/views/WorkspaceShell.vue';

const NotesStub = defineComponent({ name: 'NotesStub', render: () => h('div', { 'data-test': 'notes' }) });
const TasksStub = defineComponent({ name: 'TasksStub', render: () => h('div', { 'data-test': 'tasks' }) });
const AppRoot = defineComponent({ name: 'AppRoot', render: () => h(RouterView) });

function makeShellRouter() {
  return createRouter({
    history: createMemoryHistory(),
    routes: [
      {
        path: '/',
        component: WorkspaceShell,
        children: [
          { path: 'n/:slug?', name: 'notes', component: NotesStub },
          { path: 't/:boardId?', name: 'tasks', component: TasksStub, meta: { mobileDetail: true } },
        ],
      },
    ],
  });
}

describe('WorkspaceShell persistent sidebar', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    GET.mockResolvedValue({ data: { items: [] }, error: undefined });
    PATCH.mockResolvedValue({ data: {}, error: undefined });
    const workspace = useWorkspaceStore();
    workspace.setActiveWorkspace('atlas');
    workspace.projects = [
      { slug: 'sandbox', name: 'Sandbox', task_prefix: 'SBX', workspace_id: 'w1', visibility: 'workspace' },
    ];
  });

  it('keeps a single NotesSidebar instance mounted across Docs navigation', async () => {
    const router = makeShellRouter();
    await router.push('/n');
    await router.isReady();

    const wrapper = mount(AppRoot, { global: { plugins: [router] } });
    await flushPromises();

    const sidebarBefore = wrapper.findComponent(NotesSidebar);
    expect(sidebarBefore.exists()).toBe(true);
    expect(wrapper.find('[data-test="notes"]').exists()).toBe(true);
    // A remount would replace this DOM node; a persistent sidebar keeps it.
    const rootBefore = sidebarBefore.element;

    await router.push('/t/board-1');
    await flushPromises();

    // The routed content swapped, but the sidebar is the very same instance.
    expect(wrapper.find('[data-test="tasks"]').exists()).toBe(true);
    const sidebarAfter = wrapper.findComponent(NotesSidebar);
    expect(sidebarAfter.element).toBe(rootBefore);

    wrapper.unmount();
  });

  it('keeps the tab strip mounted across /n and /t and tracks the active tab per kind', async () => {
    const tabs = useNotesTabsStore();
    tabs.open('atlas', { kind: 'doc', id: 'note-a' }, 'Note A');
    tabs.open('atlas', { kind: 'board', id: 'board-1' }, 'Board One');

    const router = makeShellRouter();
    await router.push('/n/note-a');
    await router.isReady();

    const wrapper = mount(AppRoot, { global: { plugins: [router] } });
    await flushPromises();

    const tabNames = () => wrapper.findAll('[role="tab"]').map((t) => t.attributes('title'));
    const activeName = () =>
      wrapper
        .findAll('[role="tab"]')
        .find((t) => t.attributes('aria-selected') === 'true')
        ?.attributes('title');

    expect(tabNames()).toEqual(['Note A', 'Board One']);
    expect(activeName()).toBe('Note A');

    await router.push('/t/board-1');
    await flushPromises();

    // Same two tabs remain; the active highlight follows the route into the board.
    expect(tabNames()).toEqual(['Note A', 'Board One']);
    expect(activeName()).toBe('Board One');

    wrapper.unmount();
  });
});
