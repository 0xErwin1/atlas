import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET: vi.fn(), POST: vi.fn(), PATCH: vi.fn(), DELETE: vi.fn() },
}));

import { wrappedClient } from '@/api/wrapper';

import StatusesPanel from '@/components/settings/StatusesPanel.vue';
import { type ColumnDto, useBoardsStore } from '@/stores/boards';
import { useWorkspaceStore } from '@/stores/workspace';

const col = (id: string, name: string, positionKey: string, color: string | null = null): ColumnDto => ({
  id,
  board_id: 'board-1',
  name,
  position_key: positionKey,
  color,
  created_at: 'x',
  updated_at: 'x',
});

// Render the swatch popover inline so its buttons are queryable in the test.
const Popover = {
  template: '<div><slot name="trigger" :toggle="() => {}" /><slot :close="() => {}" /></div>',
};

function mountPanel() {
  const workspace = useWorkspaceStore();
  workspace.activeWorkspaceSlug = 'ws';
  workspace.projects = [];

  const boards = useBoardsStore();
  boards.columns = [col('c1', 'Todo', 'a'), col('c2', 'Done', 'b')];

  const wrapper = mount(StatusesPanel, {
    global: {
      stubs: {
        Dropdown: { template: '<div data-stub="dropdown" />' },
        ConfirmDialog: true,
        Popover,
      },
    },
  });

  return { wrapper, boards, workspace };
}

describe('StatusesPanel', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    // onMounted aggregates boards across projects; resolve those calls to empty.
    (wrappedClient.GET as ReturnType<typeof vi.fn>).mockResolvedValue({
      data: { items: [], has_more: false, next_cursor: null },
      error: undefined,
    });
  });

  it('recoloring a status calls updateColumn with the chosen swatch id', async () => {
    const { wrapper, boards } = mountPanel();
    // Force a board selection so the column list renders.
    (wrapper.vm as unknown as { selectedBoardId: string }).selectedBoardId = 'board-1';
    await wrapper.vm.$nextTick();

    const update = vi.spyOn(boards, 'updateColumn').mockResolvedValue(true);

    // The ColorPicker renders one button per swatch; pick "green".
    const greenSwatch = wrapper.find('button[aria-label="Green"]');
    expect(greenSwatch.exists()).toBe(true);

    await greenSwatch.trigger('click');

    expect(update).toHaveBeenCalledWith('ws', 'board-1', 'c1', { color: 'green' });
  });
});
