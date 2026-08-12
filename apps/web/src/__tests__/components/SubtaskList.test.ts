import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it } from 'vitest';
import SubtaskList from '@/components/tareas/SubtaskList.vue';

beforeEach(() => {
  setActivePinia(createPinia());
});

const subtask = (id: string, readableId: string, title: string, columnId: string, estimate?: number) => ({
  id,
  readable_id: readableId,
  board_id: 'board-1',
  column_id: columnId,
  board_name: 'Board',
  column_name: 'Todo',
  title,
  estimate,
  labels: [],
  assignees: [],
  subtask_count: 0,
  updated_at: '2026-01-01T00:00:00Z',
});

const columns = [
  { id: 'col-todo', name: 'To Do' },
  { id: 'col-done', name: 'Done' },
];

function mountList(subtasks: ReturnType<typeof subtask>[]) {
  return mount(SubtaskList, {
    props: { ws: 'atlas', subtasks, columns, boardId: 'board-1', parentReadableId: 'ATL-1' },
  });
}

describe('SubtaskList', () => {
  it('preserves the open action hint while exposing the complete sub-task title', () => {
    const wrapper = mountList([subtask('t1', 'ATL-2', 'A long sub-task title', 'col-todo')]);

    const button = wrapper.get('[data-subtask-open="t1"]');
    expect(button.attributes('title')).toBe('Open ATL-2');
    expect(button.get('span').attributes('title')).toBe('A long sub-task title');
  });

  it('renders each sub-task with its title, status and estimate', () => {
    const wrapper = mountList([subtask('t1', 'ATL-2', 'Write tests', 'col-todo', 5)]);

    const row = wrapper.get('[data-subtask="t1"]');
    expect(row.text()).toContain('Write tests');
    expect(row.text()).toContain('To Do');
    expect(row.text()).toContain('5');
    expect(row.text()).toContain('ATL-2');
  });

  it('emits open with the readable id when the title is clicked', async () => {
    const wrapper = mountList([subtask('t1', 'ATL-2', 'Write tests', 'col-todo')]);

    await wrapper.get('[data-subtask-open="t1"]').trigger('click');

    expect(wrapper.emitted('open')).toEqual([['ATL-2']]);
  });

  it('emits promote with the readable id', async () => {
    const wrapper = mountList([subtask('t1', 'ATL-2', 'Write tests', 'col-todo')]);

    await wrapper.get('[data-subtask-promote="t1"]').trigger('click');

    expect(wrapper.emitted('promote')).toEqual([['ATL-2']]);
  });

  it('emits the done column when the completion checkbox is requested', async () => {
    const wrapper = mountList([subtask('t1', 'ATL-2', 'Write tests', 'col-todo')]);
    const checkbox = wrapper.get<HTMLInputElement>('input[aria-label="Mark Write tests done"]');

    await checkbox.setValue(true);

    expect(wrapper.emitted('setColumn')).toEqual([['ATL-2', 'col-done']]);
    expect(checkbox.element.checked).toBe(false);
  });

  it('emits add with the trimmed title on enter and clears the input', async () => {
    const wrapper = mountList([]);

    const input = wrapper.get('input');
    await input.setValue('  New child  ');
    await input.trigger('keydown.enter');

    expect(wrapper.emitted('add')).toEqual([['New child']]);
    expect((input.element as HTMLInputElement).value).toBe('');
  });

  it('reads the status of a sub-task on another board from its own summary', () => {
    const foreign = {
      ...subtask('t1', 'ATL-2', 'Elsewhere', 'col-other'),
      board_id: 'board-2',
      board_name: 'Other board',
      column_name: 'In Progress',
    };

    const wrapper = mountList([foreign]);

    const row = wrapper.get('[data-subtask="t1"]');
    expect(row.text()).toContain('In Progress');
    expect(row.find('input[type="checkbox"]').exists()).toBe(false);
  });

  it('does not emit setColumn for a sub-task on another board', async () => {
    const foreign = {
      ...subtask('t1', 'ATL-2', 'Elsewhere', 'col-other'),
      board_id: 'board-2',
    };

    const wrapper = mountList([foreign]);

    expect(wrapper.find('input[aria-label="Mark Elsewhere done"]').exists()).toBe(false);
    expect(wrapper.emitted('setColumn')).toBeUndefined();
  });

  it('reveals the task search when linking an existing task', async () => {
    const wrapper = mountList([]);

    expect(wrapper.findComponent({ name: 'SearchPicker' }).exists()).toBe(false);

    await wrapper.get('[data-subtask-attach]').trigger('click');

    expect(wrapper.findComponent({ name: 'SearchPicker' }).exists()).toBe(true);
  });

  it('emits attach with the picked readable id and closes the search', async () => {
    const wrapper = mountList([]);
    await wrapper.get('[data-subtask-attach]').trigger('click');

    const picker = wrapper.findComponent({ name: 'SearchPicker' });
    picker.vm.$emit('pick', { id: 'u1', kind: 'task', title: 'Existing', readable_id: 'ATL-9' });
    await wrapper.vm.$nextTick();

    expect(wrapper.emitted('attach')).toEqual([['ATL-9']]);
    expect(wrapper.findComponent({ name: 'SearchPicker' }).exists()).toBe(false);
  });

  it('does not emit add for a blank title', async () => {
    const wrapper = mountList([]);

    const input = wrapper.get('input');
    await input.setValue('   ');
    await input.trigger('keydown.enter');

    expect(wrapper.emitted('add')).toBeUndefined();
  });
});
