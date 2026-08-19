import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { defineComponent, h } from 'vue';

vi.mock('vue-draggable-plus', () => ({
  VueDraggable: {
    name: 'VueDraggable',
    props: ['modelValue'],
    template: '<div class="vdp-stub"><slot /></div>',
  },
}));

import TaskListView from '@/components/tareas/TaskListView.vue';
import { type ColumnDto, type TaskSummaryDto, useBoardsStore } from '@/stores/boards';

/**
 * Every row holds three pickers, each with its own popover. Re-rendering a row
 * that did not change re-renders all of them, so the option lists a row receives
 * must keep their identity across an unrelated parent render.
 */

const column = (id: string, name: string, pos: string): ColumnDto => ({
  id,
  board_id: 'board-1',
  name,
  position_key: pos,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
});

const task = (id: string, columnId: string, priority: string | null): TaskSummaryDto => ({
  id,
  readable_id: `ATL-${id}`,
  board_id: 'board-1',
  column_id: columnId,
  board_name: 'Board',
  column_name: 'Todo',
  title: `Task ${id}`,
  priority,
  subtask_count: 0,
  updated_at: '2026-01-01T00:00:00Z',
});

interface RowRecord {
  renders: number;
  statusOptions: unknown;
  assigneeOptions: unknown;
  priorityOptions: unknown;
}

const rows = new Map<string, RowRecord>();

const RowStub = defineComponent({
  name: 'TaskListRow',
  props: {
    task: { type: Object, required: true },
    selected: { type: Boolean, default: false },
    done: { type: Boolean, default: false },
    ringColor: { type: String, default: '' },
    statusName: { type: String, default: '' },
    statusOptions: { type: Array, default: () => [] },
    assigneeOptions: { type: Array, default: () => [] },
    priorityOptions: { type: Array, default: () => [] },
    statusOpen: { type: Boolean, default: false },
    assigneeOpen: { type: Boolean, default: false },
    priorityOpen: { type: Boolean, default: false },
    indent: { type: Number, default: 0 },
    expandable: { type: Boolean, default: false },
    expanded: { type: Boolean, default: false },
  },
  setup(props) {
    return () => {
      const readableId = (props.task as TaskSummaryDto).readable_id;
      rows.set(readableId, {
        renders: (rows.get(readableId)?.renders ?? 0) + 1,
        statusOptions: props.statusOptions,
        assigneeOptions: props.assigneeOptions,
        priorityOptions: props.priorityOptions,
      });
      return h('div', { class: 'row-stub' }, readableId);
    };
  },
});

function seedBoard() {
  const store = useBoardsStore();
  store.columns = [column('c1', 'Backlog', 'a'), column('c2', 'Done', 'b')];
  store._setTasksForTest({
    c1: [task('1', 'c1', 'high'), task('2', 'c1', null)],
    c2: [task('3', 'c2', 'low')],
  });
  return store;
}

function mountList() {
  return mount(TaskListView, {
    props: { ws: 'ws', selectedReadableId: null },
    global: { stubs: { TaskListRow: RowStub } },
  });
}

describe('TaskListView row props', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    rows.clear();
  });

  it('renders one row per task', () => {
    seedBoard();
    const wrapper = mountList();

    expect(wrapper.findAll('.row-stub')).toHaveLength(3);
  });

  it('keeps every option list identical across an unrelated parent render', async () => {
    seedBoard();
    const wrapper = mountList();

    const before = new Map(rows);
    await wrapper.setProps({ selectedReadableId: 'ATL-missing' });

    for (const [readableId, after] of rows) {
      const previous = before.get(readableId);
      expect(after.statusOptions).toBe(previous?.statusOptions);
      expect(after.assigneeOptions).toBe(previous?.assigneeOptions);
      expect(after.priorityOptions).toBe(previous?.priorityOptions);
    }
  });

  it('does not re-render a row whose props did not change', async () => {
    seedBoard();
    const wrapper = mountList();

    const before = new Map([...rows].map(([id, record]) => [id, record.renders]));
    await wrapper.setProps({ selectedReadableId: 'ATL-missing' });

    for (const [readableId, record] of rows) {
      expect(record.renders).toBe(before.get(readableId));
    }
  });

  it('still re-renders the row that becomes selected', async () => {
    seedBoard();
    const wrapper = mountList();

    const before = rows.get('ATL-2')?.renders ?? 0;
    await wrapper.setProps({ selectedReadableId: 'ATL-2' });

    expect(rows.get('ATL-2')?.renders).toBe(before + 1);
  });

  it('rebuilds the status options when the board columns change', async () => {
    const store = seedBoard();
    mountList();

    const before = rows.get('ATL-1')?.statusOptions;
    store.columns = [column('c1', 'Renamed', 'a'), column('c2', 'Done', 'b')];
    await Promise.resolve();

    expect(rows.get('ATL-1')?.statusOptions).not.toBe(before);
  });
});
