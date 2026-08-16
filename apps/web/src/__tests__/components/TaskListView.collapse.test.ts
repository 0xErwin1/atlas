import { mount, type VueWrapper } from '@vue/test-utils';
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
import { useUiStateStore } from '@/stores/uiState';

/**
 * A collapsed group must be gone from the DOM, not merely hidden: the point of
 * collapsing a long list is to stop paying for its rows.
 */

const TaskListRowStub = defineComponent({
  name: 'TaskListRow',
  props: {
    task: { type: Object as PropType<TaskSummaryDto>, required: true },
  },
  setup(props) {
    return () => h('div', { class: 'task-row-stub', 'data-readable-id': props.task.readable_id });
  },
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

function seedBoard(): void {
  const boards = useBoardsStore();
  const ui = useUiStore();
  ui.taskGroupBy = 'status';

  boards.board = { id: 'board-1', name: 'Board' } as never;
  boards.columns = [column];
  boards._setTasksForTest({ ready: [task] });
}

function listView(): VueWrapper {
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

async function toggleGroup(wrapper: VueWrapper): Promise<void> {
  await wrapper.find('.atl-tl-grouphead').trigger('click');
  await nextTick();
}

describe('TaskListView group collapse', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    seedBoard();
  });

  it('unmounts the rows of a collapsed group instead of hiding them', async () => {
    const wrapper = listView();
    expect(wrapper.findAll('.task-row-stub')).toHaveLength(1);

    await toggleGroup(wrapper);

    expect(wrapper.findAll('.task-row-stub')).toHaveLength(0);
    expect(wrapper.find('.atl-tl-grouphead').attributes('aria-expanded')).toBe('false');
  });

  it('remembers the collapsed group per board', async () => {
    const wrapper = listView();
    await toggleGroup(wrapper);

    expect(useUiStateStore().isListGroupCollapsed('board-1', 'ready')).toBe(true);

    wrapper.unmount();
    const reopened = listView();

    expect(reopened.findAll('.task-row-stub')).toHaveLength(0);
  });

  it('leaves the groups of another board alone', async () => {
    const wrapper = listView();
    await toggleGroup(wrapper);

    const uiState = useUiStateStore();
    expect(uiState.isListGroupCollapsed('board-2', 'ready')).toBe(false);
  });
});
