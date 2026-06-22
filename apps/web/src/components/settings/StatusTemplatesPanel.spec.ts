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

describe('StatusTemplatesPanel', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('renders one row per template with its name and no board picker', async () => {
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

    const vm = wrapper.vm as unknown as {
      startEdit: (t: { id: string; name: string; color?: string | null; position_key: string }) => void;
      draftName: string;
      saveEdit: (t: {
        id: string;
        name: string;
        color?: string | null;
        position_key: string;
      }) => Promise<void>;
    };

    const target = store.templates[0];
    if (target === undefined) throw new Error('missing template fixture');

    vm.startEdit(target);
    vm.draftName = 'Backlog';
    await vm.saveEdit(target);

    expect(update).toHaveBeenCalledWith('acme', 't1', { name: 'Backlog' });
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
