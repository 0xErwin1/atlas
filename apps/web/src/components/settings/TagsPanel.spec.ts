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
