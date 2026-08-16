import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import TaskViewListView from '@/components/tareas/TaskViewListView.vue';
import type { TaskSummaryDto } from '@/stores/boards';

vi.mock('vue-router', () => ({
  useRouter: () => ({ resolve: () => ({ href: '/tasks/ATL-1' }) }),
}));

function task(readableId: string, title: string): TaskSummaryDto {
  return {
    id: readableId,
    readable_id: readableId,
    board_id: 'board-1',
    column_id: 'column-1',
    board_name: 'Board',
    column_name: 'To Do',
    title,
    priority: null,
    subtask_count: 0,
    labels: [],
    assignees: [],
    updated_at: '2026-01-01T00:00:00Z',
  } as TaskSummaryDto;
}

const feed = [task('ATL-2', 'Beta'), task('ATL-1', 'Alpha')];

function listView() {
  return mount(TaskViewListView, {
    props: { ws: 'atlas', tasks: feed, selectedReadableId: null },
    shallow: true,
  });
}

function header(wrapper: ReturnType<typeof listView>, label: string) {
  const found = wrapper.findAll('.atl-tl-hbtn').find((button) => button.text().startsWith(label));
  if (found === undefined) throw new Error(`no "${label}" column header`);
  return found;
}

function rowIds(wrapper: ReturnType<typeof listView>): string[] {
  return wrapper.findAll('.atl-tl-id-text').map((element) => element.text());
}

describe('TaskViewListView column sorting', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('starts in the order the feed arrived in', () => {
    expect(rowIds(listView())).toEqual(['ATL-2', 'ATL-1']);
  });

  it('sorts by a clicked column and reports the direction to assistive tech', async () => {
    const wrapper = listView();

    await header(wrapper, 'Name').trigger('click');

    expect(rowIds(wrapper)).toEqual(['ATL-1', 'ATL-2']);
    expect(header(wrapper, 'Name').attributes('aria-sort')).toBe('ascending');
  });

  it('reverses on the second click and un-sorts on the third', async () => {
    const wrapper = listView();
    const name = () => header(wrapper, 'Name');

    await name().trigger('click');
    await name().trigger('click');
    expect(rowIds(wrapper)).toEqual(['ATL-2', 'ATL-1']);
    expect(name().attributes('aria-sort')).toBe('descending');

    await name().trigger('click');
    expect(rowIds(wrapper)).toEqual(['ATL-2', 'ATL-1']);
    expect(name().attributes('aria-sort')).toBe('none');
  });

  it('moves the sort to the column last clicked', async () => {
    const wrapper = listView();

    await header(wrapper, 'Name').trigger('click');
    await header(wrapper, 'ID').trigger('click');

    expect(header(wrapper, 'Name').attributes('aria-sort')).toBe('none');
    expect(header(wrapper, 'ID').attributes('aria-sort')).toBe('ascending');
    expect(rowIds(wrapper)).toEqual(['ATL-1', 'ATL-2']);
  });
});
