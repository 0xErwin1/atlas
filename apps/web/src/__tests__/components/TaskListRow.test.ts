import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it } from 'vitest';
import TaskListRow from '@/components/tareas/TaskListRow.vue';
import type { TaskSummaryDto } from '@/stores/boards';

beforeEach(() => {
  setActivePinia(createPinia());
});

function makeTask(overrides: Partial<TaskSummaryDto> = {}): TaskSummaryDto {
  return {
    id: 't1',
    readable_id: 'ATL-1',
    board_id: 'board-1',
    column_id: 'col-1',
    board_name: 'Board',
    column_name: 'Todo',
    title: 'Parent task',
    priority: null,
    subtask_count: 0,
    updated_at: '2026-01-01T00:00:00Z',
    ...overrides,
  };
}

function mountRow(props: Partial<InstanceType<typeof TaskListRow>['$props']> = {}) {
  return mount(TaskListRow, {
    props: {
      task: makeTask(),
      selected: false,
      done: false,
      ringColor: 'var(--c-muted)',
      statusName: 'Todo',
      statusOptions: [],
      assigneeOptions: [],
      priorityOptions: [],
      statusOpen: false,
      assigneeOpen: false,
      priorityOpen: false,
      ...props,
    },
    global: {
      stubs: {
        // Render each picker's trigger so the row's markers/values still show.
        TaskRowPicker: { template: '<div><slot name="trigger" /></div>' },
        AssigneeAvatars: true,
        Chip: true,
        Icon: true,
      },
    },
  });
}

describe('TaskListRow', () => {
  it('renders the title and readable id', () => {
    const wrapper = mountRow();

    expect(wrapper.text()).toContain('Parent task');
    expect(wrapper.text()).toContain('ATL-1');
  });

  it('emits select with the readable id when the row is clicked', async () => {
    const wrapper = mountRow();

    await wrapper.get('.atl-tl-row').trigger('click');

    expect(wrapper.emitted('select')).toEqual([['ATL-1']]);
  });

  it('shows a disclosure caret only when the task is expandable', () => {
    expect(mountRow({ expandable: false }).find('.atl-tl-expand').exists()).toBe(false);
    expect(mountRow({ expandable: true }).find('.atl-tl-expand').exists()).toBe(true);
  });

  it('toggles expansion without opening the task when the caret is clicked', async () => {
    const wrapper = mountRow({ expandable: true });

    await wrapper.get('.atl-tl-expand').trigger('click');

    expect(wrapper.emitted('toggle-expand')?.[0]?.[0]).toMatchObject({ readable_id: 'ATL-1' });
    expect(wrapper.emitted('select')).toBeUndefined();
  });

  it('indents nested rows through the name column', () => {
    const wrapper = mountRow({ indent: 1 });

    expect(wrapper.get('.atl-tl-name').attributes('style')).toContain('padding-left: 18px');
  });
});
