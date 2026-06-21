import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it } from 'vitest';
import type { TaskSummaryDto } from '@/stores/boards';
import { useBoardsStore } from '@/stores/boards';
import { useUiStore } from '@/stores/ui';

function makeTask(overrides: Partial<TaskSummaryDto> & { id: string; column_id: string }): TaskSummaryDto {
  return {
    readable_id: overrides.id,
    board_id: 'board-1',
    board_name: 'Board',
    column_name: 'Todo',
    title: `Task ${overrides.id}`,
    priority: null,
    labels: [],
    assignees: [],
    updated_at: '2026-01-01T00:00:00Z',
    ...overrides,
  };
}

describe('useUiStore — taskFilter', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('starts with an empty filter (no active filter)', () => {
    const ui = useUiStore();

    expect(ui.taskFilter.statuses).toEqual([]);
    expect(ui.taskFilter.priorities).toEqual([]);
    expect(ui.taskFilter.assigneeIds).toEqual([]);
    expect(ui.taskFilter.labels).toEqual([]);
    expect(ui.hasActiveFilter).toBe(false);
  });

  it('hasActiveFilter is true when statuses is non-empty', () => {
    const ui = useUiStore();

    ui.setTaskFilter({ statuses: ['col-1'], priorities: [], assigneeIds: [], labels: [] });

    expect(ui.hasActiveFilter).toBe(true);
  });

  it('hasActiveFilter is true when priorities is non-empty', () => {
    const ui = useUiStore();

    ui.setTaskFilter({ statuses: [], priorities: ['high'], assigneeIds: [], labels: [] });

    expect(ui.hasActiveFilter).toBe(true);
  });

  it('hasActiveFilter is true when assigneeIds is non-empty', () => {
    const ui = useUiStore();

    ui.setTaskFilter({ statuses: [], priorities: [], assigneeIds: ['user-1'], labels: [] });

    expect(ui.hasActiveFilter).toBe(true);
  });

  it('hasActiveFilter is true when labels is non-empty', () => {
    const ui = useUiStore();

    ui.setTaskFilter({ statuses: [], priorities: [], assigneeIds: [], labels: ['bug'] });

    expect(ui.hasActiveFilter).toBe(true);
  });

  it('clearTaskFilter resets all dimensions and hasActiveFilter becomes false', () => {
    const ui = useUiStore();

    ui.setTaskFilter({ statuses: ['col-1'], priorities: ['high'], assigneeIds: ['user-1'], labels: ['bug'] });

    expect(ui.hasActiveFilter).toBe(true);

    ui.clearTaskFilter();

    expect(ui.taskFilter.statuses).toEqual([]);
    expect(ui.taskFilter.priorities).toEqual([]);
    expect(ui.taskFilter.assigneeIds).toEqual([]);
    expect(ui.taskFilter.labels).toEqual([]);
    expect(ui.hasActiveFilter).toBe(false);
  });

  it('setTaskFilter replaces the filter in full', () => {
    const ui = useUiStore();

    ui.setTaskFilter({ statuses: ['col-1'], priorities: [], assigneeIds: [], labels: [] });
    ui.setTaskFilter({ statuses: [], priorities: ['low'], assigneeIds: [], labels: [] });

    expect(ui.taskFilter.statuses).toEqual([]);
    expect(ui.taskFilter.priorities).toEqual(['low']);
  });
});

describe('useBoardsStore — filteredTasksByColumn', () => {
  const COL_A = 'col-a';
  const COL_B = 'col-b';
  const UNKNOWN = 'col-unknown';

  const taskA1 = makeTask({
    id: 'T-01',
    column_id: COL_A,
    priority: 'high',
    labels: ['bug', 'frontend'],
    assignees: [{ id: 'user-1', type: 'user', display_name: 'Alice' }],
  });

  const taskA2 = makeTask({
    id: 'T-02',
    column_id: COL_A,
    priority: 'low',
    labels: ['backend'],
    assignees: [{ id: 'user-2', type: 'user', display_name: 'Bob' }],
  });

  const taskB1 = makeTask({
    id: 'T-03',
    column_id: COL_B,
    priority: 'urgent',
    labels: [],
    assignees: [],
  });

  const taskA3 = makeTask({
    id: 'T-04',
    column_id: COL_A,
    priority: null,
    labels: ['bug'],
    assignees: [
      { id: 'user-1', type: 'user', display_name: 'Alice' },
      { id: 'user-3', type: 'user', display_name: 'Carol' },
    ],
  });

  function seedTasks(boards: ReturnType<typeof useBoardsStore>): void {
    boards._setTasksForTest({
      [COL_A]: [taskA1, taskA2, taskA3],
      [COL_B]: [taskB1],
    });
  }

  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('returns all tasks when no filter is active', () => {
    const boards = useBoardsStore();
    seedTasks(boards);

    const result = boards.filteredTasksByColumn(COL_A);

    expect(result).toHaveLength(3);
    expect(result.map((t) => t.readable_id)).toEqual(['T-01', 'T-02', 'T-04']);
  });

  it('returns an empty array for an unknown column', () => {
    const boards = useBoardsStore();
    seedTasks(boards);

    const result = boards.filteredTasksByColumn(UNKNOWN);

    expect(result).toEqual([]);
  });

  it('filters by status (column_id) — keeps only tasks in the selected column', () => {
    const boards = useBoardsStore();
    const ui = useUiStore();
    seedTasks(boards);

    ui.setTaskFilter({ statuses: [COL_A], priorities: [], assigneeIds: [], labels: [] });

    const resultA = boards.filteredTasksByColumn(COL_A);
    const resultB = boards.filteredTasksByColumn(COL_B);

    expect(resultA).toHaveLength(3);
    expect(resultB).toHaveLength(0);
  });

  it('filters by priority — single value', () => {
    const boards = useBoardsStore();
    const ui = useUiStore();
    seedTasks(boards);

    ui.setTaskFilter({ statuses: [], priorities: ['high'], assigneeIds: [], labels: [] });

    const result = boards.filteredTasksByColumn(COL_A);

    expect(result).toHaveLength(1);
    expect(result[0]?.readable_id).toBe('T-01');
  });

  it('filters by priority — OR within dimension (high OR low)', () => {
    const boards = useBoardsStore();
    const ui = useUiStore();
    seedTasks(boards);

    ui.setTaskFilter({ statuses: [], priorities: ['high', 'low'], assigneeIds: [], labels: [] });

    const result = boards.filteredTasksByColumn(COL_A);

    expect(result).toHaveLength(2);
    expect(result.map((t) => t.readable_id)).toEqual(['T-01', 'T-02']);
  });

  it('filters by label — task passes if any of its labels is in the selected set (OR)', () => {
    const boards = useBoardsStore();
    const ui = useUiStore();
    seedTasks(boards);

    ui.setTaskFilter({ statuses: [], priorities: [], assigneeIds: [], labels: ['bug'] });

    const result = boards.filteredTasksByColumn(COL_A);

    expect(result).toHaveLength(2);
    expect(result.map((t) => t.readable_id)).toEqual(['T-01', 'T-04']);
  });

  it('filters by label — tasks with no labels are excluded when label filter is active', () => {
    const boards = useBoardsStore();
    const ui = useUiStore();
    seedTasks(boards);

    ui.setTaskFilter({ statuses: [], priorities: [], assigneeIds: [], labels: ['bug'] });

    const result = boards.filteredTasksByColumn(COL_B);

    expect(result).toHaveLength(0);
  });

  it('filters by assigneeId — task passes if any assignee id is in the selected set (OR)', () => {
    const boards = useBoardsStore();
    const ui = useUiStore();
    seedTasks(boards);

    ui.setTaskFilter({ statuses: [], priorities: [], assigneeIds: ['user-2'], labels: [] });

    const result = boards.filteredTasksByColumn(COL_A);

    expect(result).toHaveLength(1);
    expect(result[0]?.readable_id).toBe('T-02');
  });

  it('filters by assigneeId — task with multiple assignees passes if any match', () => {
    const boards = useBoardsStore();
    const ui = useUiStore();
    seedTasks(boards);

    ui.setTaskFilter({ statuses: [], priorities: [], assigneeIds: ['user-3'], labels: [] });

    const result = boards.filteredTasksByColumn(COL_A);

    expect(result).toHaveLength(1);
    expect(result[0]?.readable_id).toBe('T-04');
  });

  it('applies AND across dimensions — priority AND label', () => {
    const boards = useBoardsStore();
    const ui = useUiStore();
    seedTasks(boards);

    ui.setTaskFilter({ statuses: [], priorities: ['high'], assigneeIds: [], labels: ['bug'] });

    const result = boards.filteredTasksByColumn(COL_A);

    expect(result).toHaveLength(1);
    expect(result[0]?.readable_id).toBe('T-01');
  });

  it('applies AND across dimensions — assignee AND label', () => {
    const boards = useBoardsStore();
    const ui = useUiStore();
    seedTasks(boards);

    ui.setTaskFilter({ statuses: [], priorities: [], assigneeIds: ['user-1'], labels: ['bug'] });

    const result = boards.filteredTasksByColumn(COL_A);

    expect(result).toHaveLength(2);
    expect(result.map((t) => t.readable_id)).toEqual(['T-01', 'T-04']);
  });

  it('returns empty result when filter matches nothing', () => {
    const boards = useBoardsStore();
    const ui = useUiStore();
    seedTasks(boards);

    ui.setTaskFilter({ statuses: [], priorities: ['urgent'], assigneeIds: [], labels: [] });

    const result = boards.filteredTasksByColumn(COL_A);

    expect(result).toHaveLength(0);
  });

  it('handles tasks with null labels (undefined-safe)', () => {
    const boards = useBoardsStore();
    const ui = useUiStore();

    const taskNoLabels = makeTask({ id: 'T-05', column_id: COL_A, labels: undefined });
    boards._setTasksForTest({ [COL_A]: [taskNoLabels] });

    ui.setTaskFilter({ statuses: [], priorities: [], assigneeIds: [], labels: ['bug'] });

    const result = boards.filteredTasksByColumn(COL_A);

    expect(result).toHaveLength(0);
  });

  it('handles tasks with no assignees (undefined-safe)', () => {
    const boards = useBoardsStore();
    const ui = useUiStore();

    const taskNoAssignees = makeTask({ id: 'T-06', column_id: COL_A, assignees: undefined });
    boards._setTasksForTest({ [COL_A]: [taskNoAssignees] });

    ui.setTaskFilter({ statuses: [], priorities: [], assigneeIds: ['user-1'], labels: [] });

    const result = boards.filteredTasksByColumn(COL_A);

    expect(result).toHaveLength(0);
  });

  // Identity stability — the kanban draggable freezes in an infinite render loop
  // if its bound list changes reference on every render. The no-filter path must
  // return the stable raw array, and the filtered path must be memoized so the
  // reference only changes when the raw tasks or the active filter change.
  it('returns the stable raw reference when no filter is active', () => {
    const boards = useBoardsStore();
    seedTasks(boards);

    const a = boards.filteredTasksByColumn(COL_A);
    const b = boards.filteredTasksByColumn(COL_A);

    expect(a).toBe(b);
    expect(a).toBe(boards.tasksByColumn(COL_A));
  });

  it('returns the same reference across repeated calls while raw and filter are unchanged', () => {
    const boards = useBoardsStore();
    const ui = useUiStore();
    seedTasks(boards);

    ui.setTaskFilter({ statuses: [], priorities: ['high'], assigneeIds: [], labels: [] });

    const a = boards.filteredTasksByColumn(COL_A);
    const b = boards.filteredTasksByColumn(COL_A);

    expect(a).toBe(b);
  });

  it('returns a new reference when the filter changes', () => {
    const boards = useBoardsStore();
    const ui = useUiStore();
    seedTasks(boards);

    ui.setTaskFilter({ statuses: [], priorities: ['high'], assigneeIds: [], labels: [] });
    const a = boards.filteredTasksByColumn(COL_A);

    ui.setTaskFilter({ statuses: [], priorities: ['low'], assigneeIds: [], labels: [] });
    const b = boards.filteredTasksByColumn(COL_A);

    expect(a).not.toBe(b);
    expect(b.map((t) => t.readable_id)).toEqual(['T-02']);
  });

  it('returns a new reference when the raw tasks change', () => {
    const boards = useBoardsStore();
    const ui = useUiStore();
    seedTasks(boards);

    ui.setTaskFilter({ statuses: [], priorities: ['high'], assigneeIds: [], labels: [] });
    const a = boards.filteredTasksByColumn(COL_A);

    boards._setTasksForTest({ [COL_A]: [taskA1, taskA2, taskA3], [COL_B]: [taskB1] });
    const b = boards.filteredTasksByColumn(COL_A);

    expect(a).not.toBe(b);
  });
});
