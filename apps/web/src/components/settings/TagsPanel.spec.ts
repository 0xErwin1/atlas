import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import TagsPanel from '@/components/settings/TagsPanel.vue';
import { useTagsStore } from '@/stores/tags';
import { useWorkspaceStore } from '@/stores/workspace';

function setupStore() {
  const workspace = useWorkspaceStore();
  workspace.activeWorkspaceSlug = 'acme';

  const tagsStore = useTagsStore();
  tagsStore.tags = [{ id: 'tag-1', name: 'urgent', color: 'red' }] as never;
  vi.spyOn(tagsStore, 'load').mockResolvedValue(undefined as never);
  vi.spyOn(tagsStore, 'loadUsed').mockResolvedValue(undefined as never);

  return { workspace, tagsStore };
}

describe('TagsPanel — edit mode folds name + color', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('has no standalone recolor trigger on a row', async () => {
    setupStore();
    const wrapper = mount(TagsPanel);
    await wrapper.vm.$nextTick();

    expect(wrapper.find('.atl-tag-row').exists()).toBe(true);
    expect(wrapper.find('.atl-tag-swatch-btn').exists()).toBe(false);
    expect(wrapper.find('.atl-color-trigger').exists()).toBe(false);
  });

  it('renders the color picker inline in edit mode, not inside a popover trigger', async () => {
    setupStore();
    const wrapper = mount(TagsPanel);
    await wrapper.vm.$nextTick();

    (wrapper.vm as unknown as { startRename: (id: string, name: string) => void }).startRename(
      'tag-1',
      'urgent',
    );
    await wrapper.vm.$nextTick();

    expect(wrapper.find('.atl-tag-row.editing').exists()).toBe(true);
    expect(wrapper.find('.atl-color-trigger').exists()).toBe(false);
    expect(wrapper.find('.atl-edit-picker').exists()).toBe(true);
    expect(wrapper.find('.atl-edit-picker').classes()).toContain('color-picker');
    expect(wrapper.find('.atl-edit-picker .hex-text').exists()).toBe(true);
  });

  it('selecting a hex updates the draft color without leaving edit mode', async () => {
    setupStore();
    const wrapper = mount(TagsPanel);
    await wrapper.vm.$nextTick();

    const vm = wrapper.vm as unknown as {
      startRename: (id: string, name: string) => void;
      draftColor: string;
      editingId: string | null;
    };

    vm.startRename('tag-1', 'urgent');
    await wrapper.vm.$nextTick();

    const hexInput = wrapper.find('.atl-edit-picker .hex-text');
    await hexInput.setValue('#FF0000');

    expect(vm.draftColor).toBe('#FF0000');
    expect(vm.editingId).toBe('tag-1');
    expect(wrapper.find('.atl-edit-picker').exists()).toBe(true);
  });

  it('Save applies both the edited name and the picked color in one update call', async () => {
    const { tagsStore } = setupStore();
    const update = vi.spyOn(tagsStore, 'update').mockResolvedValue(true);

    const wrapper = mount(TagsPanel);
    await wrapper.vm.$nextTick();

    const vm = wrapper.vm as unknown as {
      startRename: (id: string, name: string) => void;
      draftName: string;
      draftColor: string;
      saveEdit: (id: string, name: string) => Promise<void>;
    };

    vm.startRename('tag-1', 'urgent');
    await wrapper.vm.$nextTick();

    vm.draftName = 'critical';
    vm.draftColor = '#FF0000';
    await vm.saveEdit('tag-1', 'urgent');

    expect(update).toHaveBeenCalledWith('acme', 'tag-1', { name: 'critical', color: '#FF0000' });
  });
});

describe('TagsPanel — used-but-unregistered tier', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('renders unregistered labels in a separate tier', async () => {
    const { tagsStore } = setupStore();
    tagsStore.usedLabels = ['backend', 'urgent'];

    const wrapper = mount(TagsPanel);
    await wrapper.vm.$nextTick();

    const used = wrapper.findAll('.atl-used-row');
    expect(used).toHaveLength(1);
    expect(used[0]?.text()).toContain('backend');
  });

  it('the Register action creates the used label in the registry', async () => {
    const { tagsStore } = setupStore();
    tagsStore.usedLabels = ['backend'];
    const create = vi.spyOn(tagsStore, 'create').mockResolvedValue({
      id: 'tag-2',
      name: 'backend',
      color: null,
    } as never);

    const wrapper = mount(TagsPanel);
    await wrapper.vm.$nextTick();

    await wrapper.find('.atl-used-row .atl-register-btn').trigger('click');

    expect(create).toHaveBeenCalledWith('acme', 'backend');
  });
});
