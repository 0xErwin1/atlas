import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET, PATCH, POST, DELETE } = vi.hoisted(() => ({
  GET: vi.fn(),
  PATCH: vi.fn(),
  POST: vi.fn(),
  DELETE: vi.fn(),
}));

vi.mock('vue-router', () => ({
  useRoute: () => ({ params: {} }),
  useRouter: () => ({ push: vi.fn() }),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET, PATCH, POST, DELETE },
}));

const { useLiveUpdates } = vi.hoisted(() => ({ useLiveUpdates: vi.fn() }));
vi.mock('@/composables/useLiveUpdates', () => ({ useLiveUpdates }));

import NotesSpace from '@/components/notas/NotesSpace.vue';
import LoadingState from '@/components/states/LoadingState.vue';
import { useWorkspaceStore } from '@/stores/workspace';
import NotesSidebar from '@/views/NotesSidebar.vue';

function setupProjects() {
  const workspace = useWorkspaceStore();
  workspace.setActiveWorkspace('atlas');
  workspace.projects = [
    { slug: 'sandbox', name: 'Sandbox', task_prefix: 'SBX', workspace_id: 'w1', visibility: 'workspace' },
    { slug: 'roadmap', name: 'Roadmap', task_prefix: 'RD', workspace_id: 'w1', visibility: 'workspace' },
  ];
  return workspace;
}

describe('NotesSidebar whole-sidebar loading gate', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    PATCH.mockResolvedValue({ data: {}, error: undefined });
    try {
      localStorage.clear();
    } catch {
      // jsdom always provides localStorage; ignore if absent
    }
  });

  it('shows a single loader and hides the tree until every space has settled', async () => {
    setupProjects();

    let releaseGet: () => void = () => {};
    const pending = new Promise<void>((resolve) => {
      releaseGet = resolve;
    });
    GET.mockReturnValue(pending.then(() => ({ data: { items: [] }, error: undefined })));

    const wrapper = mount(NotesSidebar);
    await flushPromises();

    // Gate closed: the sidebar's own loader is present and the tree is hidden.
    expect(wrapper.findComponent(LoadingState).exists()).toBe(true);
    expect(wrapper.find('.notes-sidebar-body').attributes('style')).toContain('display: none');

    releaseGet();
    await flushPromises();

    // Gate open: the tree is revealed and the single gate loader is gone.
    const tree = wrapper.find('.notes-sidebar-body');
    expect(tree.attributes('style') ?? '').not.toContain('display: none');
    expect(wrapper.findComponent(LoadingState).exists()).toBe(false);
    expect(wrapper.findAllComponents(NotesSpace)).toHaveLength(2);

    wrapper.unmount();
  });

  it('does not reopen the gate on a later revalidation', async () => {
    setupProjects();
    GET.mockResolvedValue({ data: { items: [] }, error: undefined });

    const wrapper = mount(NotesSidebar);
    await flushPromises();

    expect(wrapper.find('.notes-sidebar-body').attributes('style') ?? '').not.toContain('display: none');

    // A background revalidation re-announcing readiness must not close the gate.
    wrapper.findAllComponents(NotesSpace)[0]?.vm.$emit('initial-settled');
    await flushPromises();

    expect(wrapper.find('.notes-sidebar-body').attributes('style') ?? '').not.toContain('display: none');
    expect(wrapper.findComponent(LoadingState).exists()).toBe(false);

    wrapper.unmount();
  });

  it('preserves settled projects when a later project refresh adds another space', async () => {
    const workspace = setupProjects();
    GET.mockResolvedValue({ data: { items: [] }, error: undefined });
    const wrapper = mount(NotesSidebar);
    await flushPromises();

    let releaseGet: () => void = () => {};
    const pending = new Promise<void>((resolve) => {
      releaseGet = resolve;
    });
    GET.mockReturnValueOnce(pending.then(() => ({ data: { items: [] }, error: undefined })));
    GET.mockReturnValueOnce(pending.then(() => ({ data: { items: [] }, error: undefined })));
    GET.mockReturnValueOnce(pending.then(() => ({ data: { items: [] }, error: undefined })));
    workspace.projects = [
      ...workspace.projects,
      {
        slug: 'new-space',
        name: 'New space',
        task_prefix: 'NEW',
        workspace_id: 'w1',
        visibility: 'workspace',
      },
    ];
    await flushPromises();

    expect(wrapper.findComponent(LoadingState).exists()).toBe(true);
    releaseGet();
    await flushPromises();

    expect(wrapper.findComponent(LoadingState).exists()).toBe(false);
    expect(wrapper.find('.notes-sidebar-body').attributes('style') ?? '').not.toContain('display: none');
    wrapper.unmount();
  });
});
