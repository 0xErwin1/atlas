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

  it('renders a "New project" create footer', async () => {
    setupProjects();
    const wrapper = mount(NotesSidebar);
    await flushPromises();

    const footer = wrapper.find('button[aria-label="New project"]');
    expect(footer.exists()).toBe(true);
    expect(footer.text()).toContain('New project');

    await footer.trigger('click');
    const footerMenu = wrapper.findAllComponents(ContextMenu).find((menu) => {
      const labels = (menu.props('items') as Array<{ label?: string }>).map((item) => item.label);
      return labels.includes('New project');
    });
    expect(footerMenu).toBeDefined();
    expect(footerMenu?.props('items')).toEqual([
      { label: 'New project', icon: 'folder-plus', action: expect.any(Function) },
    ]);
    wrapper.unmount();
  });

  it('keeps views and creation outside the scrollable sidebar content', async () => {
    setupProjects();
    const wrapper = mount(NotesSidebar);
    await flushPromises();

    const content = wrapper.get('[role="region"][aria-label="Sidebar content"]');
    const footer = wrapper.get('footer[aria-label="Sidebar actions"]');

    expect(content.text()).toContain('Spaces');
    expect(footer.text()).toContain('Views');
    expect(footer.get('button[aria-label="New project"]').text()).toContain('New project');
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

  it('opens the shared project-create menu from empty background', async () => {
    const workspace = useWorkspaceStore();
    workspace.setActiveWorkspace('atlas');
    workspace.projects = [];
    const wrapper = mount(NotesSidebar);
    await flushPromises();

    const backgroundEvent = new MouseEvent('contextmenu', { bubbles: true, cancelable: true });
    wrapper.element.dispatchEvent(backgroundEvent);
    await wrapper.vm.$nextTick();

    const menu = wrapper.findAllComponents(ContextMenu).find((candidate) => candidate.props('open') === true);
    expect(menu?.props('items')).toEqual([
      { label: 'New project', icon: 'folder-plus', action: expect.any(Function) },
    ]);
    expect(backgroundEvent.defaultPrevented).toBe(true);
    wrapper.unmount();
  });

  it('preserves native context menus for sidebar rows and interactive targets', async () => {
    setupProjects();
    const wrapper = mount(NotesSidebar);
    await flushPromises();

    const targets = [
      Object.assign(document.createElement('button'), { className: 'atl-row' }),
      document.createElement('input'),
      document.createElement('textarea'),
      (() => {
        const editor = document.createElement('span');
        editor.setAttribute('contenteditable', 'true');
        return editor;
      })(),
      document.createElement('button'),
      Object.assign(document.createElement('a'), { href: '#' }),
    ];
    for (const target of targets) {
      wrapper.element.append(target);
      const event = new MouseEvent('contextmenu', { bubbles: true, cancelable: true });
      target.dispatchEvent(event);
      expect(event.defaultPrevented).toBe(false);
      target.remove();
    }
    wrapper.unmount();
  });
});
