import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { defineComponent, h, nextTick, type PropType } from 'vue';

vi.mock('vue-draggable-plus', () => ({
  VueDraggable: {
    name: 'VueDraggable',
    props: ['modelValue'],
    template: '<div class="vdp-stub"><slot /></div>',
  },
}));

import TaskListView from '@/components/tareas/TaskListView.vue';
import { type ColumnDto, type TaskSummaryDto, useBoardsStore } from '@/stores/boards';
import { useUiStore } from '@/stores/ui';

const TaskListRowStub = defineComponent({
  name: 'TaskListRow',
  props: {
    task: { type: Object as PropType<TaskSummaryDto>, required: true },
    selected: { type: Boolean, required: true },
    done: { type: Boolean, required: true },
    ringColor: { type: String, required: true },
    statusName: { type: String, required: true },
    statusOptions: { type: Array, required: true },
    assigneeOptions: { type: Array, required: true },
    priorityOptions: { type: Array, required: true },
    statusOpen: { type: Boolean, required: true },
    assigneeOpen: { type: Boolean, required: true },
    priorityOpen: { type: Boolean, required: true },
    indent: { type: Number, default: 0 },
    expandable: { type: Boolean, default: false },
    expanded: { type: Boolean, default: false },
  },
  emits: ['toggle-expand', 'status-pick'],
  setup(props, { emit }) {
    return () =>
      h('div', [
        h(
          'button',
          {
            class: 'task-row-stub',
            'data-readable-id': props.task.readable_id,
            'data-ring-color': props.ringColor,
            'data-status-name': props.statusName,
            'data-indent': props.indent,
            onClick: () => {
              if (props.expandable) emit('toggle-expand', props.task);
            },
          },
          props.statusName,
        ),
        h(
          'button',
          {
            class: 'status-pick-stub',
            'data-readable-id': props.task.readable_id,
            onClick: () => emit('status-pick', props.task, 'progress'),
          },
          'Pick progress',
        ),
      ]);
  },
});

const column = (id: string, name: string, color: string): ColumnDto => ({
  id,
  board_id: 'board-1',
  name,
  position_key: id,
  color,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
});

const task = (
  id: string,
  readableId: string,
  columnId: string,
  columnName: string,
  subtaskCount = 0,
): TaskSummaryDto => ({
  id,
  readable_id: readableId,
  board_id: 'board-1',
  column_id: columnId,
  board_name: 'Board',
  column_name: columnName,
  title: `Task ${id}`,
  priority: null,
  subtask_count: subtaskCount,
  labels: [],
  assignees: [],
  updated_at: '2026-01-01T00:00:00Z',
});

async function expandParent(wrapper: VueWrapper): Promise<void> {
  await wrapper.find('.task-row-stub[data-readable-id="ATL-1"]').trigger('click');
  await nextTick();
  await nextTick();
}

describe('TaskListView sub-task rows', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('updates a cached sub-task row after a successful status move', async () => {
    const boards = useBoardsStore();
    const ui = useUiStore();
    ui.taskGroupBy = 'status';

    boards.columns = [column('ready', 'Ready to take', 'amber'), column('progress', 'In Progress', 'blue')];
    boards._setTasksForTest({
      ready: [task('parent-id', 'ATL-1', 'ready', 'Ready to take', 1)],
      progress: [],
    });
    const loadSubtasks = vi
      .spyOn(boards, 'loadSubtasks')
      .mockResolvedValue([task('child-id', 'ATL-2', 'ready', 'Ready to take')]);
    const moveTaskToColumn = vi.spyOn(boards, 'moveTaskToColumn').mockResolvedValue(true);

    const wrapper = mount(TaskListView, {
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

    await expandParent(wrapper);

    expect(loadSubtasks).toHaveBeenCalledWith('ws', 'ATL-1');
    const childRow = wrapper.find('.task-row-stub[data-readable-id="ATL-2"]');
    expect(childRow.attributes('data-status-name')).toBe('Ready to take');
    expect(childRow.attributes('data-ring-color')).toBe('var(--c-primary)');

    await wrapper.find('.status-pick-stub[data-readable-id="ATL-2"]').trigger('click');
    await flushPromises();
    await nextTick();

    expect(moveTaskToColumn).toHaveBeenCalledWith('ws', 'ATL-2', 'progress');
    const updatedChildRow = wrapper.find('.task-row-stub[data-readable-id="ATL-2"]');
    expect(updatedChildRow.attributes('data-status-name')).toBe('In Progress');
    expect(updatedChildRow.attributes('data-ring-color')).toBe('var(--c-info)');
  });
});
