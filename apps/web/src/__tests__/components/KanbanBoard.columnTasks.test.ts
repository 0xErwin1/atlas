import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('vue-draggable-plus', () => ({
  VueDraggable: {
    name: 'VueDraggable',
    props: ['modelValue'],
    template: '<div class="vdp-stub"><slot /></div>',
  },
}));

import KanbanBoard from '@/components/tareas/KanbanBoard.vue';
import KanbanColumn from '@/components/tareas/KanbanColumn.vue';
import { type ColumnDto, type TaskSummaryDto, useBoardsStore } from '@/stores/boards';

/**
 * The board renders one column component per column and hands each its task
 * list. Recomputing that list on every board render re-filters every task and
 * hands each column a fresh array, which re-renders every card underneath it.
 */

const column = (id: string, name: string, pos: string): ColumnDto => ({
  id,
  board_id: 'board-1',
  name,
  position_key: pos,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
});

const task = (id: string, columnId: string): TaskSummaryDto => ({
  id,
  readable_id: `ATL-${id}`,
  board_id: 'board-1',
  column_id: columnId,
  board_name: 'Board',
  column_name: 'Todo',
  title: `Task ${id}`,
  priority: null,
  subtask_count: 0,
  updated_at: '2026-01-01T00:00:00Z',
});

function seedBoard() {
  const store = useBoardsStore();
  store.columns = [column('c1', 'Backlog', 'a'), column('c2', 'Done', 'b')];
  store._setTasksForTest({ c1: [task('1', 'c1'), task('2', 'c1')], c2: [task('3', 'c2')] });
  return store;
}

describe('KanbanBoard column tasks', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('gives each column its filtered tasks', () => {
    seedBoard();
    const wrapper = mount(KanbanBoard, { props: { ws: 'ws' } });

    const columns = wrapper.findAllComponents(KanbanColumn);
    expect(columns[0]?.props('tasks')).toHaveLength(2);
    expect(columns[1]?.props('tasks')).toHaveLength(1);
  });

  it('does not re-filter the columns on an unrelated board render', async () => {
    const store = seedBoard();
    const wrapper = mount(KanbanBoard, { props: { ws: 'ws' } });

    const filtered = vi.spyOn(store, 'filteredTasksByColumn');
    await wrapper.setProps({ selectedReadableId: 'ATL-missing' });

    expect(filtered).not.toHaveBeenCalled();
  });

  it('keeps each column task list identical across an unrelated board render', async () => {
    seedBoard();
    const wrapper = mount(KanbanBoard, { props: { ws: 'ws' } });

    const before = wrapper.findAllComponents(KanbanColumn).map((c) => c.props('tasks'));
    await wrapper.setProps({ selectedReadableId: 'ATL-missing' });
    const after = wrapper.findAllComponents(KanbanColumn).map((c) => c.props('tasks'));

    expect(after[0]).toBe(before[0]);
    expect(after[1]).toBe(before[1]);
  });

  it('re-filters when the board tasks change', async () => {
    const store = seedBoard();
    const wrapper = mount(KanbanBoard, { props: { ws: 'ws' } });

    store._setTasksForTest({ c1: [task('1', 'c1')], c2: [task('3', 'c2')] });
    await wrapper.vm.$nextTick();

    expect(wrapper.findAllComponents(KanbanColumn)[0]?.props('tasks')).toHaveLength(1);
  });
});
