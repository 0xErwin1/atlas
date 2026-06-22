import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import StatusesPanel from '@/components/settings/StatusesPanel.vue';
import { useBoardsStore } from '@/stores/boards';
import { useWorkspaceStore } from '@/stores/workspace';

function setupStore() {
  const workspace = useWorkspaceStore();
  workspace.activeWorkspaceSlug = 'acme';
  workspace.projects = [];
  vi.spyOn(workspace, 'loadProjects').mockResolvedValue(undefined as never);

  const boards = useBoardsStore();
  boards.columns = [{ id: 'col-1', name: 'Todo', color: 'blue', position_key: 'a0' }] as never;
  vi.spyOn(boards, 'loadBoardsForProject').mockResolvedValue(undefined as never);
  vi.spyOn(boards, 'loadColumns').mockResolvedValue(undefined as never);

  return { workspace, boards };
}

describe('StatusesPanel — edit mode folds name + color', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('has no standalone recolor trigger on a row (color is static outside edit mode)', async () => {
    setupStore();
    const wrapper = mount(StatusesPanel);
    (wrapper.vm as unknown as { selectedBoardId: string }).selectedBoardId = 'board-1';
    await wrapper.vm.$nextTick();

    expect(wrapper.find('.atl-status-row').exists()).toBe(true);
    expect(wrapper.find('.atl-dot-btn').exists()).toBe(false);
    expect(wrapper.find('.atl-color-trigger').exists()).toBe(false);
  });

  it('Save applies both the edited name and the picked color in one update call', async () => {
    const { boards } = setupStore();
    const updateColumn = vi.spyOn(boards, 'updateColumn').mockResolvedValue(true);

    const wrapper = mount(StatusesPanel);
    (wrapper.vm as unknown as { selectedBoardId: string }).selectedBoardId = 'board-1';
    await wrapper.vm.$nextTick();

    const vm = wrapper.vm as unknown as {
      startRename: (c: { id: string; name: string; color?: string }) => void;
      draftName: string;
      draftColor: string;
      saveEdit: (c: { id: string; name: string }) => Promise<void>;
    };

    vm.startRename({ id: 'col-1', name: 'Todo', color: 'blue' });
    await wrapper.vm.$nextTick();

    vm.draftName = 'In progress';
    vm.draftColor = '#1A2B3C';
    await vm.saveEdit({ id: 'col-1', name: 'Todo' });

    expect(updateColumn).toHaveBeenCalledWith('acme', 'board-1', 'col-1', {
      name: 'In progress',
      color: '#1A2B3C',
    });
  });
});
