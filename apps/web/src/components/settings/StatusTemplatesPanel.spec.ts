import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import StatusTemplatesPanel from '@/components/settings/StatusTemplatesPanel.vue';
import { useStatusTemplatesStore } from '@/stores/statusTemplates';
import { useWorkspaceStore } from '@/stores/workspace';

function tpl(over: Record<string, unknown> = {}) {
  return {
    id: 't1',
    workspace_id: 'ws1',
    name: 'Todo',
    color: null,
    position_key: 'a',
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    ...over,
  };
}

function setup() {
  const workspace = useWorkspaceStore();
  workspace.activeWorkspaceSlug = 'acme';
  workspace.projects = [];
  vi.spyOn(workspace, 'loadProjects').mockResolvedValue(undefined);

  const store = useStatusTemplatesStore();
  vi.spyOn(store, 'load').mockResolvedValue(undefined);
  store.templates = [
    tpl({ id: 't1', name: 'Todo', position_key: 'a' }),
    tpl({ id: 't2', name: 'Doing', position_key: 'b' }),
  ] as never;

  return { workspace, store };
}

/** Opens the edit row for the template at `index` and returns its rename input. */
async function startEditing(wrapper: ReturnType<typeof mount>, index: number) {
  const row = wrapper.findAll('.atl-status-row')[index];
  if (row === undefined) throw new Error(`missing row ${index}`);

  const editAction = row.findAll('button').find((b) => b.attributes('title') === 'Edit name & color');
  if (editAction === undefined) throw new Error('missing edit action');
  await editAction.trigger('click');

  return wrapper.find('.atl-status-row.editing .atl-status-rename');
}

describe('StatusTemplatesPanel', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('renders one row per template with its name', async () => {
    setup();

    const wrapper = mount(StatusTemplatesPanel);
    await wrapper.vm.$nextTick();

    const rows = wrapper.findAll('.atl-status-row');
    expect(rows).toHaveLength(2);
    expect(rows[0]?.text()).toContain('Todo');
    expect(rows[1]?.text()).toContain('Doing');
  });

  it('saving an edit calls update with the changed fields', async () => {
    const { store } = setup();
    const update = vi.spyOn(store, 'update').mockResolvedValue(true);

    const wrapper = mount(StatusTemplatesPanel);
    await wrapper.vm.$nextTick();

    const nameInput = await startEditing(wrapper, 0);
    await nameInput.setValue('Backlog');
    await nameInput.trigger('keydown.enter');

    expect(update).toHaveBeenCalledWith('acme', 't1', { name: 'Backlog' });
  });

  it('renders the color picker inline in edit mode and a hex selection persists on save', async () => {
    const { store } = setup();
    const update = vi.spyOn(store, 'update').mockResolvedValue(true);

    const wrapper = mount(StatusTemplatesPanel);
    await wrapper.vm.$nextTick();

    const nameInput = await startEditing(wrapper, 0);

    expect(wrapper.find('.atl-color-trigger').exists()).toBe(false);
    expect(wrapper.find('.atl-edit-picker').classes()).toContain('color-picker');

    const hexInput = wrapper.find('.atl-edit-picker .hex-text');
    await hexInput.setValue('#0A0B0C');
    await nameInput.trigger('keydown.enter');

    expect(update).toHaveBeenCalledWith('acme', 't1', { color: '#0A0B0C' });
  });

  it('moving a row down asks the store for the position after its next sibling', async () => {
    const { store } = setup();
    const move = vi.spyOn(store, 'move').mockResolvedValue(true);

    const wrapper = mount(StatusTemplatesPanel);
    await wrapper.vm.$nextTick();

    const firstRow = wrapper.findAll('.atl-status-row')[0];
    if (firstRow === undefined) throw new Error('missing first row');

    const moveDown = firstRow.findAll('button').find((b) => b.attributes('title') === 'Move down');
    if (moveDown === undefined) throw new Error('missing move-down action');
    await moveDown.trigger('click');

    expect(move).toHaveBeenCalledWith('acme', 't1', { before: 'b', after: null });
  });

  it('applying to the selected board calls applyToBoard', async () => {
    const { store } = setup();
    const apply = vi.spyOn(store, 'applyToBoard').mockResolvedValue(true);

    const wrapper = mount(StatusTemplatesPanel);
    await wrapper.vm.$nextTick();

    const vm = wrapper.vm as unknown as {
      selectedBoardId: string;
      applyToBoard: () => Promise<void>;
    };

    vm.selectedBoardId = 'board-9';
    await vm.applyToBoard();

    expect(apply).toHaveBeenCalledWith('acme', 'board-9');
  });
});
