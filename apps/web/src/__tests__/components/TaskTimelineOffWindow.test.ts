import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import TaskTimelineView from '@/components/tareas/TaskTimelineView.vue';
import { type ColumnDto, type TaskDto, type TaskSummaryDto, useBoardsStore } from '@/stores/boards';

vi.mock('vue-router', () => ({
  useRouter: () => ({ resolve: () => ({ href: '/tasks/ATL-1' }) }),
}));

/**
 * The timeline shows a window around today. Anything outside it has to be
 * counted, not dropped — an overdue task is the one a timeline is most often
 * opened to find.
 */

const column: ColumnDto = {
  id: 'column-1',
  board_id: 'board-1',
  name: 'In Progress',
  position_key: 'a0',
  color: 'blue',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
};

function task(readableId: string): TaskSummaryDto {
  return {
    id: readableId,
    readable_id: readableId,
    board_id: 'board-1',
    column_id: 'column-1',
    board_name: 'Board',
    column_name: 'In Progress',
    title: `Task ${readableId}`,
    priority: null,
    subtask_count: 0,
    labels: [],
    assignees: [],
    updated_at: '2026-01-01T00:00:00Z',
  } as TaskSummaryDto;
}

function daysFromToday(days: number): string {
  const date = new Date();
  date.setDate(date.getDate() + days);
  return date.toISOString();
}

function timelineWithDueDates(dueByReadableId: Record<string, string | null>) {
  const boards = useBoardsStore();
  boards.columns = [column];
  boards._setTasksForTest({ 'column-1': Object.keys(dueByReadableId).map(task) });
  vi.spyOn(boards, 'taskDetail').mockImplementation(
    (readableId: string) => ({ due_date: dueByReadableId[readableId] ?? null }) as TaskDto,
  );

  return mount(TaskTimelineView, {
    props: { ws: 'atlas', selectedReadableId: null },
    shallow: true,
  });
}

describe('TaskTimelineView off-window tasks', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('counts an overdue task instead of dropping it', () => {
    const wrapper = timelineWithDueDates({ 'ATL-1': daysFromToday(-60) });

    const note = wrapper.get('.atl-tm-note');
    expect(note.text()).toContain('1 overdue task before this range');
    expect(note.classes()).toContain('overdue');
  });

  it('separates overdue, later and undated tasks', () => {
    const wrapper = timelineWithDueDates({
      'ATL-1': daysFromToday(-60),
      'ATL-2': daysFromToday(60),
      'ATL-3': null,
    });

    const note = wrapper.get('.atl-tm-note').text();
    expect(note).toContain('1 overdue task before this range');
    expect(note).toContain('1 due after it');
    expect(note).toContain('1 with no due date');
  });

  it('says nothing when every task fits on the axis', () => {
    const wrapper = timelineWithDueDates({ 'ATL-1': daysFromToday(1) });

    expect(wrapper.find('.atl-tm-note').exists()).toBe(false);
  });
});
