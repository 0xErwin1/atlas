import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { defineComponent, h, type PropType } from 'vue';

vi.mock('vue-draggable-plus', () => ({
  VueDraggable: {
    name: 'VueDraggable',
    props: ['modelValue', 'disabled'],
    template: '<div class="vdp-stub" :data-disabled="String(disabled)"><slot /></div>',
  },
}));

import KanbanColumn from '@/components/tareas/KanbanColumn.vue';
import TaskListView from '@/components/tareas/TaskListView.vue';
import { type ColumnDto, type TaskSummaryDto, useBoardsStore } from '@/stores/boards';
import { useUiStore } from '@/stores/ui';

/**
 * An archived board is read-only server-side. The views drop their write
 * affordances so the refusal is visible before the click, not after it.
 */

const TaskListRowStub = defineComponent({
  name: 'TaskListRow',
  props: { task: { type: Object as PropType<TaskSummaryDto>, required: true } },
  setup: (props) => () => h('div', { 'data-readable-id': props.task.readable_id }),
});

const column: ColumnDto = {
  id: 'ready',
  board_id: 'board-1',
  name: 'Ready',
  position_key: 'a0',
  color: 'amber',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
};

const task: TaskSummaryDto = {
  id: 'task-1',
  readable_id: 'ATL-1',
  board_id: 'board-1',
  column_id: 'ready',
  board_name: 'Board',
  column_name: 'Ready',
  title: 'Task',
  priority: null,
  subtask_count: 0,
  labels: [],
  assignees: [],
  updated_at: '2026-01-01T00:00:00Z',
};

function seedBoard(archived: boolean): void {
  const boards = useBoardsStore();
  const ui = useUiStore();
  ui.taskGroupBy = 'status';

  boards.board = {
    id: 'board-1',
    name: 'Board',
    ...(archived ? { archived_at: '2026-08-16T00:00:00Z' } : {}),
  } as never;
  boards.columns = [column];
  boards._setTasksForTest({ ready: [task] });
}

function listView() {
  return mount(TaskListView, {
    props: { ws: 'ws', selectedReadableId: null },
    global: {
      stubs: {
        TaskListRow: TaskListRowStub,
        ConfirmDialog: true,
        ContextMenu: true,
        Icon: true,
        PromptDialog: true,
      },
    },
  });
}

function kanbanColumn(readOnly: boolean) {
  return mount(KanbanColumn, {
    props: { column, tasks: [task], selectedReadableId: null, readOnly },
    global: {
      stubs: { TaskCard: true, ContextMenu: true, ColorPicker: true, Icon: true, Btn: true },
    },
  });
}

describe('an archived board', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('hides the inline add-task row in the list view', () => {
    seedBoard(false);
    expect(listView().find('.atl-tl-add-row').exists()).toBe(true);

    setActivePinia(createPinia());
    seedBoard(true);
    expect(listView().find('.atl-tl-add-row').exists()).toBe(false);
  });

  it('hides the kanban add-task button and freezes card dragging', () => {
    seedBoard(true);
    const wrapper = kanbanColumn(true);

    expect(wrapper.find('[aria-label="Add task"]').exists()).toBe(false);
    expect(wrapper.find('.vdp-stub').attributes('data-disabled')).toBe('true');
  });

  it('leaves the affordances of an open board alone', () => {
    seedBoard(false);
    const wrapper = kanbanColumn(false);

    expect(wrapper.find('[aria-label="Add task"]').exists()).toBe(true);
    expect(wrapper.find('.vdp-stub').attributes('data-disabled')).toBe('false');
  });
});
