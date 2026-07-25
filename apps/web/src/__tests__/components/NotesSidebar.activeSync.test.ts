import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET, PATCH, POST, DELETE } = vi.hoisted(() => ({
  GET: vi.fn(),
  PATCH: vi.fn(),
  POST: vi.fn(),
  DELETE: vi.fn(),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET, PATCH, POST, DELETE },
}));

const { useLiveUpdates } = vi.hoisted(() => ({ useLiveUpdates: vi.fn() }));
vi.mock('@/composables/useLiveUpdates', () => ({ useLiveUpdates }));

// A reactive route stand-in: navigation is simulated by assigning through the
// exported proxy, which is what makes the sidebar's computed highlights update.
vi.mock('vue-router', async () => {
  const { reactive } = await import('vue');
  const route = reactive({ params: {} as Record<string, string> });
  return {
    useRoute: () => route,
    useRouter: () => ({ push: vi.fn() }),
    __route: route,
  };
});

import * as vueRouter from 'vue-router';
import { boardKey, docKey } from '@/lib/notesTree';
import { useTreeSelection } from '@/stores/treeSelection';
import { useWorkspaceStore } from '@/stores/workspace';
import NotesSidebar from '@/views/NotesSidebar.vue';

const route = (vueRouter as unknown as { __route: { params: Record<string, string> } }).__route;

async function navigateTo(params: Record<string, string>): Promise<void> {
  route.params = params;
  await flushPromises();
}

function setupProjects() {
  const workspace = useWorkspaceStore();
  workspace.setActiveWorkspace('atlas');
  workspace.projects = [
    { slug: 'sandbox', name: 'Sandbox', task_prefix: 'SBX', workspace_id: 'w1', visibility: 'workspace' },
  ];
  return workspace;
}

describe('NotesSidebar tree selection follows the active node', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    GET.mockResolvedValue({ data: { items: [] }, error: undefined });
    PATCH.mockResolvedValue({ data: {}, error: undefined });
    route.params = {};
  });

  it('moves the selection to the newly active board when switching between boards', async () => {
    setupProjects();
    const selection = useTreeSelection();
    const wrapper = mount(NotesSidebar);
    await navigateTo({ boardId: 'board-a' });

    expect(selection.keys()).toEqual([boardKey('board-a')]);

    await navigateTo({ boardId: 'board-b' });

    // The previous board must not stay highlighted alongside the active one.
    expect(selection.keys()).toEqual([boardKey('board-b')]);

    wrapper.unmount();
  });

  it('selects the board when navigating from a document to a board', async () => {
    setupProjects();
    const selection = useTreeSelection();
    const wrapper = mount(NotesSidebar);
    await navigateTo({ slug: 'a-note' });

    expect(selection.keys()).toEqual([docKey('a-note')]);

    await navigateTo({ boardId: 'board-a' });

    expect(selection.keys()).toEqual([boardKey('board-a')]);

    wrapper.unmount();
  });

  it('clears the selection when neither a document nor a board is open', async () => {
    setupProjects();
    const selection = useTreeSelection();
    const wrapper = mount(NotesSidebar);
    await navigateTo({ boardId: 'board-a' });

    await navigateTo({});

    expect(selection.keys()).toEqual([]);

    wrapper.unmount();
  });
});
