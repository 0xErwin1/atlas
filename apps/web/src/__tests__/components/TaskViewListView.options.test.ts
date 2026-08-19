import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import TaskRowPicker from '@/components/tareas/TaskRowPicker.vue';
import TaskViewListView from '@/components/tareas/TaskViewListView.vue';
import type { TaskSummaryDto } from '@/stores/boards';

vi.mock('vue-router', () => ({
  useRouter: () => ({ resolve: () => ({ href: '/tasks/ATL-1' }) }),
}));

/**
 * The cross-board list builds its pickers inline in the template, so a fresh
 * option array per render re-renders every picker and its popover.
 */

function task(readableId: string, priority: string | null): TaskSummaryDto {
  return {
    id: readableId,
    readable_id: readableId,
    board_id: 'board-1',
    column_id: 'column-1',
    board_name: 'Board',
    column_name: 'To Do',
    title: readableId,
    priority,
    subtask_count: 0,
    labels: [],
    assignees: [],
    updated_at: '2026-01-01T00:00:00Z',
  } as TaskSummaryDto;
}

const feed = [task('ATL-1', 'high'), task('ATL-2', null)];

function listView() {
  return mount(TaskViewListView, {
    props: { ws: 'atlas', tasks: feed, selectedReadableId: null },
    shallow: true,
  });
}

function pickerOptions(wrapper: ReturnType<typeof listView>): unknown[] {
  return wrapper.findAllComponents(TaskRowPicker).map((picker) => picker.props('options'));
}

describe('TaskViewListView picker options', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('renders a picker per row', () => {
    expect(pickerOptions(listView()).length).toBeGreaterThan(0);
  });

  it('keeps the option lists identical across an unrelated re-render', async () => {
    const wrapper = listView();

    const before = pickerOptions(wrapper);
    await wrapper.setProps({ selectedReadableId: 'ATL-missing' });
    const after = pickerOptions(wrapper);

    expect(after).toHaveLength(before.length);
    for (const [index, options] of after.entries()) {
      expect(options).toBe(before[index]);
    }
  });
});
