import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import TaskCard from '@/components/tareas/TaskCard.vue';
import TaskTableView from '@/components/tareas/TaskTableView.vue';
import TaskTimelineView from '@/components/tareas/TaskTimelineView.vue';
import TaskViewListView from '@/components/tareas/TaskViewListView.vue';
import { type ColumnDto, type TaskDto, type TaskSummaryDto, useBoardsStore } from '@/stores/boards';

vi.mock('vue-router', () => ({
  useRouter: () => ({ resolve: () => ({ href: '/tasks/ATL-96' }) }),
}));

// The timeline renders a window around *today*, so a fixture pinned to a fixed
// calendar date stops producing a bar as soon as that date leaves the window —
// and the assertion below would fail for a reason that has nothing to do with
// truncated titles. Both dates are therefore relative to the run.
const TODAY = new Date().toISOString();

const task: TaskSummaryDto = {
  id: 'task-96',
  readable_id: 'ATL-96',
  board_id: 'board-1',
  column_id: 'column-1',
  board_name: 'Readability',
  column_name: 'In Progress',
  title: 'A complete task title that remains available when the visible label truncates',
  priority: null,
  subtask_count: 0,
  labels: [],
  assignees: [],
  updated_at: TODAY,
};

const column: ColumnDto = {
  id: 'column-1',
  board_id: 'board-1',
  name: 'In Progress',
  position_key: 'a0',
  color: 'blue',
  created_at: '2026-07-27T00:00:00Z',
  updated_at: '2026-07-27T00:00:00Z',
};

function mountTaskView(component: typeof TaskTableView | typeof TaskTimelineView) {
  return mount(component, {
    props: { ws: 'atlas', selectedReadableId: null },
    shallow: true,
  });
}

describe('truncated title contracts', () => {
  beforeEach(() => {
    setActivePinia(createPinia());

    const boards = useBoardsStore();
    boards.columns = [column];
    boards._setTasksForTest({ [column.id]: [task] });
    vi.spyOn(boards, 'taskDetail').mockReturnValue({ due_date: TODAY } as TaskDto);
  });

  it('renders the complete task title on each confirmed ellipsized task view surface', () => {
    const list = mount(TaskViewListView, {
      props: { ws: 'atlas', tasks: [task], selectedReadableId: null },
      shallow: true,
    });
    const table = mountTaskView(TaskTableView);
    const timeline = mountTaskView(TaskTimelineView);

    expect(list.get('.atl-tl-title').text()).toBe(task.title);
    expect(list.get('.atl-tl-title').attributes('title')).toBe(task.title);
    expect(table.get('.atl-tt-title').text()).toBe(task.title);
    expect(table.get('.atl-tt-title').attributes('title')).toBe(task.title);
    expect(timeline.get('.atl-tm-title').text()).toBe(task.title);
    expect(timeline.get('.atl-tm-title').attributes('title')).toBe(task.title);
  });

  it('leaves a representative excluded non-ellipsized task title without a title attribute', () => {
    const card = mount(TaskCard, { props: { task }, shallow: true });
    const title = card.get('.atl-task-card > span');

    expect(title.text()).toBe(task.title);
    expect(title.attributes('title')).toBeUndefined();
  });
});
