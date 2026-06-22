import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ProjectsPanel from '@/components/settings/ProjectsPanel.vue';
import { useWorkspaceStore } from '@/stores/workspace';

function setupStore() {
  const workspace = useWorkspaceStore();
  workspace.activeWorkspaceSlug = 'acme';
  workspace.projects = [
    { slug: 'atlas', name: 'Atlas', task_prefix: 'ATL', workspace_id: 'ws1' },
    { slug: 'backend', name: 'Backend', task_prefix: 'BE', workspace_id: 'ws1' },
  ];

  vi.spyOn(workspace, 'loadProjects').mockResolvedValue(undefined);

  return workspace;
}

describe('ProjectsPanel — project list', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('renders one row per project with its name and prefix', async () => {
    setupStore();

    const wrapper = mount(ProjectsPanel);
    await wrapper.vm.$nextTick();

    const rows = wrapper.findAll('.atl-proj-row');
    expect(rows).toHaveLength(2);

    const firstRow = rows[0];
    expect(firstRow?.text()).toContain('Atlas');
    expect(firstRow?.text()).toContain('ATL');
  });

  it('shows an empty state when there are no projects', async () => {
    const workspace = useWorkspaceStore();
    workspace.activeWorkspaceSlug = 'acme';
    workspace.projects = [];
    vi.spyOn(workspace, 'loadProjects').mockResolvedValue(undefined);

    const wrapper = mount(ProjectsPanel);
    await wrapper.vm.$nextTick();

    expect(wrapper.find('.atl-proj-empty').exists()).toBe(true);
    expect(wrapper.find('.atl-proj-list').exists()).toBe(false);
  });
});

describe('ProjectsPanel — edit flow', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('calls updateProject with changed name and prefix on save', async () => {
    const workspace = setupStore();
    const update = vi.spyOn(workspace, 'updateProject').mockResolvedValue(true);

    const wrapper = mount(ProjectsPanel);
    await wrapper.vm.$nextTick();

    await wrapper.find('.atl-rowact').trigger('click');
    await wrapper.vm.$nextTick();

    const vm = wrapper.vm as unknown as {
      draftName: string;
      draftPrefix: string;
      saveEdit: (slug: string) => Promise<void>;
    };

    vm.draftName = 'Atlas Renamed';
    vm.draftPrefix = 'ATLX';
    await vm.saveEdit('atlas');

    expect(update).toHaveBeenCalledWith('acme', 'atlas', { name: 'Atlas Renamed', task_prefix: 'ATLX' });
  });

  it('shows a note about new-tasks-only prefix semantics in edit mode', async () => {
    setupStore();

    const wrapper = mount(ProjectsPanel);
    await wrapper.vm.$nextTick();

    await wrapper.find('.atl-rowact').trigger('click');
    await wrapper.vm.$nextTick();

    expect(wrapper.find('.atl-prefix-note').exists()).toBe(true);
    expect(wrapper.find('.atl-prefix-note').text()).toContain('existing IDs keep their prefix');
  });

  it('validates the prefix format client-side before calling the store', async () => {
    const workspace = setupStore();
    const update = vi.spyOn(workspace, 'updateProject').mockResolvedValue(true);

    const wrapper = mount(ProjectsPanel);
    await wrapper.vm.$nextTick();

    await wrapper.find('.atl-rowact').trigger('click');
    await wrapper.vm.$nextTick();

    const vm = wrapper.vm as unknown as {
      draftName: string;
      draftPrefix: string;
      saveEdit: (slug: string) => Promise<void>;
      prefixError: string | null;
    };

    vm.draftName = 'Atlas';
    vm.draftPrefix = 'bad prefix';
    await vm.saveEdit('atlas');

    expect(update).not.toHaveBeenCalled();
    expect(vm.prefixError).not.toBeNull();
  });
});
