import { describe, expect, it } from 'vitest';
import { defaultDirectionFor, sortTasks, type TaskSortKey } from '@/lib/taskSort';
import type { TaskSummaryDto } from '@/stores/boards';

function task(overrides: Partial<TaskSummaryDto> & { readable_id: string }): TaskSummaryDto {
  return {
    id: overrides.readable_id,
    board_id: 'board-1',
    column_id: 'column-1',
    board_name: 'Board',
    column_name: 'To Do',
    title: 'Task',
    priority: null,
    subtask_count: 0,
    labels: [],
    assignees: [],
    updated_at: '2026-01-01T00:00:00Z',
    ...overrides,
  } as TaskSummaryDto;
}

function ids(tasks: TaskSummaryDto[]): string[] {
  return tasks.map((t) => t.readable_id);
}

function sorted(tasks: TaskSummaryDto[], key: TaskSortKey): string[] {
  return ids(sortTasks(tasks, key, defaultDirectionFor(key)));
}

describe('sortTasks', () => {
  it('leaves the feed alone when no column is sorted', () => {
    const feed = [task({ readable_id: 'ATL-3' }), task({ readable_id: 'ATL-1' })];

    expect(ids(sortTasks(feed, null, 'asc'))).toEqual(['ATL-3', 'ATL-1']);
  });

  it('orders readable ids by number, not by text', () => {
    const feed = [
      task({ readable_id: 'ATL-10' }),
      task({ readable_id: 'ATL-9' }),
      task({ readable_id: 'BRS-2' }),
    ];

    expect(sorted(feed, 'id')).toEqual(['ATL-9', 'ATL-10', 'BRS-2']);
  });

  it('sorts priority by urgency, most urgent first', () => {
    const feed = [
      task({ readable_id: 'ATL-1', priority: 'low' }),
      task({ readable_id: 'ATL-2', priority: 'urgent' }),
      task({ readable_id: 'ATL-3', priority: 'medium' }),
    ];

    expect(sorted(feed, 'priority')).toEqual(['ATL-2', 'ATL-3', 'ATL-1']);
  });

  it('keeps tasks with nothing to order last in both directions', () => {
    const feed = [
      task({ readable_id: 'ATL-1' }),
      task({ readable_id: 'ATL-2', estimate: 5 }),
      task({ readable_id: 'ATL-3', estimate: 1 }),
    ];

    expect(ids(sortTasks(feed, 'estimate', 'desc'))).toEqual(['ATL-2', 'ATL-3', 'ATL-1']);
    expect(ids(sortTasks(feed, 'estimate', 'asc'))).toEqual(['ATL-3', 'ATL-2', 'ATL-1']);
  });

  it('sorts the most recently updated first', () => {
    const feed = [
      task({ readable_id: 'ATL-1', updated_at: '2026-01-01T00:00:00Z' }),
      task({ readable_id: 'ATL-2', updated_at: '2026-06-01T00:00:00Z' }),
    ];

    expect(sorted(feed, 'updated')).toEqual(['ATL-2', 'ATL-1']);
  });

  it('sorts names case-insensitively', () => {
    const feed = [
      task({ readable_id: 'ATL-1', title: 'beta' }),
      task({ readable_id: 'ATL-2', title: 'Alpha' }),
    ];

    expect(sorted(feed, 'name')).toEqual(['ATL-2', 'ATL-1']);
  });

  it('keeps the incoming order between rows that compare equal', () => {
    const feed = [
      task({ readable_id: 'ATL-3', title: 'Same' }),
      task({ readable_id: 'ATL-1', title: 'Same' }),
      task({ readable_id: 'ATL-2', title: 'Same' }),
    ];

    expect(sorted(feed, 'name')).toEqual(['ATL-3', 'ATL-1', 'ATL-2']);
  });

  it('never mutates the feed it was given', () => {
    const feed = [task({ readable_id: 'ATL-2' }), task({ readable_id: 'ATL-1' })];

    sortTasks(feed, 'id', 'asc');

    expect(ids(feed)).toEqual(['ATL-2', 'ATL-1']);
  });

  it('reads text columns from A and value columns from the top', () => {
    expect(defaultDirectionFor('name')).toBe('asc');
    expect(defaultDirectionFor('id')).toBe('asc');
    expect(defaultDirectionFor('priority')).toBe('desc');
    expect(defaultDirectionFor('estimate')).toBe('desc');
    expect(defaultDirectionFor('updated')).toBe('desc');
  });
});
