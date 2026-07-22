import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import ProjectCreateDialog from '@/components/projects/ProjectCreateDialog.vue';
import ProjectsPanel from '@/components/settings/ProjectsPanel.vue';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

function setupStore() {
  const workspace = useWorkspaceStore();
  workspace.activeWorkspaceSlug = 'acme';
  workspace.projects = [
    { slug: 'atlas', name: 'Atlas', task_prefix: 'ATL', workspace_id: 'ws1', visibility: 'workspace' },
    { slug: 'backend', name: 'Backend', task_prefix: 'BE', workspace_id: 'ws1', visibility: 'workspace' },
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

interface DeleteVm {
  deleteTarget: { slug: string; name: string } | null;
  confirmDelete: () => Promise<void>;
}

describe('ProjectsPanel — create flow', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('opens the create dialog from the New project button', async () => {
    setupStore();

    const wrapper = mount(ProjectsPanel);
    await wrapper.vm.$nextTick();

    expect((wrapper.vm as unknown as { createOpen: boolean }).createOpen).toBe(false);

    await wrapper.find('.atl-panel-head-actions button').trigger('click');

    expect((wrapper.vm as unknown as { createOpen: boolean }).createOpen).toBe(true);
  });

  it('calls createProject and closes the dialog on success', async () => {
    const workspace = setupStore();
    const create = vi.spyOn(workspace, 'createProject').mockResolvedValue('marketing');

    const wrapper = mount(ProjectsPanel);
    await wrapper.vm.$nextTick();

    const dialog = wrapper.findComponent(ProjectCreateDialog);
    await (dialog.vm as unknown as { submit: (name: string) => Promise<void> }).submit('Marketing');

    expect(create).toHaveBeenCalledWith('acme', 'Marketing');
    expect((wrapper.vm as unknown as { createOpen: boolean }).createOpen).toBe(false);
  });

  it('blocks an empty name without calling the store', async () => {
    const workspace = setupStore();
    const create = vi.spyOn(workspace, 'createProject').mockResolvedValue('x');

    const wrapper = mount(ProjectsPanel);
    await wrapper.vm.$nextTick();

    const dialog = wrapper.findComponent(ProjectCreateDialog);
    await (dialog.vm as unknown as { submit: (name: string) => Promise<void> }).submit('   ');

    expect(create).not.toHaveBeenCalled();
    expect((dialog.vm as unknown as { error: string }).error).not.toBe('');
  });

  it('surfaces a store error in the dialog on failure', async () => {
    const workspace = setupStore();
    vi.spyOn(workspace, 'createProject').mockImplementation(async () => {
      workspace.error = 'Prefix already in use';
      return null;
    });

    const wrapper = mount(ProjectsPanel);
    await wrapper.vm.$nextTick();

    const dialog = wrapper.findComponent(ProjectCreateDialog);
    (wrapper.vm as unknown as { createOpen: boolean }).createOpen = true;
    await (dialog.vm as unknown as { submit: (name: string) => Promise<void> }).submit('Marketing');

    expect((dialog.vm as unknown as { error: string }).error).toBe('Prefix already in use');
    expect((wrapper.vm as unknown as { createOpen: boolean }).createOpen).toBe(true);
  });
});

describe('ProjectsPanel — delete flow', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('renders a delete action per project row', async () => {
    setupStore();

    const wrapper = mount(ProjectsPanel);
    await wrapper.vm.$nextTick();

    expect(wrapper.findAll('.atl-rowact--danger')).toHaveLength(2);
  });

  it('opens the confirm dialog for the targeted project', async () => {
    setupStore();

    const wrapper = mount(ProjectsPanel);
    await wrapper.vm.$nextTick();

    const firstDelete = wrapper.findAll('.atl-rowact--danger')[0];
    await firstDelete?.trigger('click');

    expect((wrapper.vm as unknown as DeleteVm).deleteTarget?.slug).toBe('atlas');
  });

  it('calls deleteProject after confirmation and clears the target', async () => {
    const workspace = setupStore();
    const del = vi.spyOn(workspace, 'deleteProject').mockResolvedValue(true);

    const wrapper = mount(ProjectsPanel);
    await wrapper.vm.$nextTick();

    const vm = wrapper.vm as unknown as DeleteVm;
    vm.deleteTarget = { slug: 'atlas', name: 'Atlas' };
    await vm.confirmDelete();

    expect(del).toHaveBeenCalledWith('acme', 'atlas');
    expect(vm.deleteTarget).toBeNull();
  });

  it('surfaces a store error when the delete fails', async () => {
    const workspace = setupStore();
    vi.spyOn(workspace, 'deleteProject').mockImplementation(async () => {
      workspace.error = 'Not permitted';
      return false;
    });
    const banner = vi.spyOn(useUiStore(), 'showBanner');

    const wrapper = mount(ProjectsPanel);
    await wrapper.vm.$nextTick();

    const vm = wrapper.vm as unknown as DeleteVm;
    vm.deleteTarget = { slug: 'atlas', name: 'Atlas' };
    await vm.confirmDelete();

    expect(banner).toHaveBeenCalledWith('Not permitted', 'error');
  });
});
