import { flushPromises, mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET, PATCH, POST, DELETE } = vi.hoisted(() => ({
  GET: vi.fn(),
  PATCH: vi.fn(),
  POST: vi.fn(),
  DELETE: vi.fn(),
}));

const { push } = vi.hoisted(() => ({ push: vi.fn() }));

vi.mock('vue-router', () => ({
  useRoute: () => ({ params: {} }),
  useRouter: () => ({ push }),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET, PATCH, POST, DELETE },
}));

const { useLiveUpdates } = vi.hoisted(() => ({ useLiveUpdates: vi.fn() }));
vi.mock('@/composables/useLiveUpdates', () => ({ useLiveUpdates }));

import NotesSpace from '@/components/notas/NotesSpace.vue';
import SidebarViews from '@/components/notas/SidebarViews.vue';
import ContextMenu from '@/components/ui/ContextMenu.vue';
import Dropdown from '@/components/ui/Dropdown.vue';
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

describe('NotesSidebar unified all-projects container', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    GET.mockResolvedValue({ data: { items: [] }, error: undefined });
    PATCH.mockResolvedValue({ data: {}, error: undefined });
    try {
      localStorage.clear();
    } catch {
      // jsdom always provides localStorage; ignore if absent
    }
  });

  it('renders every accessible project as its own SPACE section, with no per-project dropdown', async () => {
    setupProjects();
    const wrapper = mount(NotesSidebar);
    await flushPromises();

    expect(wrapper.text()).toContain('Spaces');
    const spaces = wrapper.findAllComponents(NotesSpace);
    expect(spaces).toHaveLength(2);
    expect(spaces.map((s) => s.props('project').slug)).toEqual(['sandbox', 'roadmap']);
    expect(wrapper.text()).toContain('Sandbox');
    expect(wrapper.text()).toContain('Roadmap');

    expect(wrapper.findComponent(Dropdown).exists()).toBe(false);
    wrapper.unmount();
  });

  it('relocates the VIEWS block and activates a predefined view on selection', async () => {
    setupProjects();
    const wrapper = mount(NotesSidebar);
    await flushPromises();

    const views = wrapper.findComponent(SidebarViews);
    expect(views.exists()).toBe(true);
    expect(wrapper.text()).toContain('Views');
    expect(wrapper.text()).toContain('My tasks');

    const myTasks = wrapper.findAll('button').find((b) => b.text().includes('My tasks'));
    await myTasks?.trigger('click');

    expect(push).toHaveBeenCalledWith({ name: 'task-view', params: { viewId: 'my-tasks' } });
    wrapper.unmount();
  });

  it('renders a "New page or board" create footer', async () => {
    setupProjects();
    const wrapper = mount(NotesSidebar);
    await flushPromises();

    const footer = wrapper.find('button[aria-label="New page or board"]');
    expect(footer.exists()).toBe(true);
    expect(footer.text()).toContain('New page or board');

    await footer.trigger('click');
    const labels = (wrapper.findComponent(ContextMenu).props('items') as Array<{ label?: string }>).map(
      (i) => i.label,
    );
    expect(labels).toContain('New page');
    expect(labels).toContain('New board');
    wrapper.unmount();
  });

  it('shows an empty message when the workspace has no projects', async () => {
    const workspace = useWorkspaceStore();
    workspace.setActiveWorkspace('atlas');
    workspace.projects = [];
    const wrapper = mount(NotesSidebar);
    await flushPromises();

    expect(wrapper.findAllComponents(NotesSpace)).toHaveLength(0);
    expect(wrapper.text()).toContain('No projects yet.');
    wrapper.unmount();
  });
});
