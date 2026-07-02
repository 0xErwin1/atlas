import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET, POST, PATCH, DELETE } = vi.hoisted(() => ({
  GET: vi.fn(),
  POST: vi.fn(),
  PATCH: vi.fn(),
  DELETE: vi.fn(),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET, POST, PATCH, DELETE },
}));

import { useTaskInteractions } from '@/composables/useTaskInteractions';
import type { ColumnDto, TaskSummaryDto } from '@/stores/boards';
import { useBoardsStore } from '@/stores/boards';

const col = (id: string, name: string): ColumnDto => ({
  id,
  board_id: 'board-1',
  name,
  position_key: id,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
});

const task = (id: string, readableId: string, columnId: string, boardId = 'board-1'): TaskSummaryDto => ({
  id,
  readable_id: readableId,
  board_id: boardId,
  column_id: columnId,
  board_name: 'Board',
  column_name: 'Todo',
  title: `Task ${id}`,
  priority: null,
  updated_at: '2026-01-01T00:00:00Z',
});

const columns = [col('c1', 'Todo'), col('c2', 'In Progress'), col('c3', 'Done')];

describe('useTaskInteractions.buildMenuItems', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('returns the full action set (incl. Add tag) for a single-board context', () => {
    const ti = useTaskInteractions('ws');
    const t = task('t1', 'AB-1', 'c1');
    const items = ti.buildMenuItems({
      task: t,
      boardId: 'board-1',
      columns,
      allowDuplicate: true,
      onOpen: () => {},
    });

    const labels = items.filter((it) => it.sep !== true).map((it) => it.label);

    expect(labels).toContain('Open');
    expect(labels).toContain('Open in new tab');
    expect(labels).toContain('Rename');
    expect(labels).toContain('Change status');
    expect(labels).toContain('Change priority');
    expect(labels).toContain('Assign to');
    expect(labels).toContain('Move to board');
    expect(labels).toContain('Set due date');
    expect(labels).toContain('Add tag');
    expect(labels).toContain('Copy ID');
    expect(labels).toContain('Copy link');
    expect(labels).toContain('Duplicate');
    expect(labels).toContain('Delete');
  });

  it('Add tag prompt appends a label to the task via updateTask', async () => {
    PATCH.mockResolvedValueOnce({ data: task('t1', 'AB-1', 'c1'), error: undefined });

    const ti = useTaskInteractions('ws');
    ti.menuReadableId.value = 'AB-1';
    ti.openAddTag();
    await ti.onPromptConfirm('needs-review');

    expect(PATCH).toHaveBeenCalledTimes(1);
    const [, opts] = PATCH.mock.calls[0] as [string, { body: { labels: string[] } }];
    expect(opts.body.labels).toEqual(['needs-review']);
    expect(ti.promptState.value.open).toBe(false);
  });

  it('Add tag ignores a blank value', async () => {
    const ti = useTaskInteractions('ws');
    ti.menuReadableId.value = 'AB-1';
    ti.openAddTag();
    await ti.onPromptConfirm('   ');

    expect(PATCH).not.toHaveBeenCalled();
  });

  it('status submenu uses ctx.columns — not the boards store', () => {
    const ti = useTaskInteractions('ws');
    const t = task('t1', 'AB-1', 'c1');

    const ctxColumns = [col('cx1', 'Custom A'), col('cx2', 'Custom B')];
    const items = ti.buildMenuItems({
      task: t,
      boardId: 'board-1',
      columns: ctxColumns,
      allowDuplicate: true,
      onOpen: () => {},
    });

    const changeStatus = items.find((it) => it.label === 'Change status');
    expect(changeStatus).toBeDefined();
    const childLabels = changeStatus?.children?.map((c) => c.label) ?? [];
    expect(childLabels).toContain('Custom A');
    expect(childLabels).toContain('Custom B');
    expect(childLabels).not.toContain('Todo');
  });

  it('current column is disabled in status submenu', () => {
    const ti = useTaskInteractions('ws');
    const t = task('t1', 'AB-1', 'c1');
    const items = ti.buildMenuItems({
      task: t,
      boardId: 'board-1',
      columns,
      allowDuplicate: true,
      onOpen: () => {},
    });

    const changeStatus = items.find((it) => it.label === 'Change status');
    const todoItem = changeStatus?.children?.find((c) => c.label === 'Todo');
    expect(todoItem?.disabled).toBe(true);

    const inProgressItem = changeStatus?.children?.find((c) => c.label === 'In Progress');
    expect(inProgressItem?.disabled).toBeFalsy();
  });

  it('Duplicate is enabled when allowDuplicate is true', () => {
    const ti = useTaskInteractions('ws');
    const t = task('t1', 'AB-1', 'c1');
    const items = ti.buildMenuItems({
      task: t,
      boardId: 'board-1',
      columns,
      allowDuplicate: true,
      onOpen: () => {},
    });

    const dup = items.find((it) => it.label === 'Duplicate');
    expect(dup).toBeDefined();
    expect(dup?.disabled).toBeFalsy();
  });

  it('Duplicate is disabled when allowDuplicate is false', () => {
    const ti = useTaskInteractions('ws');
    const t = task('t1', 'AB-1', 'c1');
    const items = ti.buildMenuItems({
      task: t,
      boardId: undefined,
      columns,
      allowDuplicate: false,
      onOpen: () => {},
    });

    const dup = items.find((it) => it.label === 'Duplicate');
    expect(dup?.disabled).toBe(true);
  });

  it('cross-board task: boardId from task.board_id enables Duplicate', () => {
    const ti = useTaskInteractions('ws');
    const t = task('t1', 'AB-1', 'c1', 'other-board');

    const items = ti.buildMenuItems({
      task: t,
      boardId: t.board_id,
      columns,
      allowDuplicate: t.board_id !== undefined && t.board_id !== '',
      onOpen: () => {},
    });

    const dup = items.find((it) => it.label === 'Duplicate');
    expect(dup?.disabled).toBeFalsy();
  });

  it('single-board task: ctx.boardId from store filters Move-to-board submenu', () => {
    const store = useBoardsStore();
    const ti = useTaskInteractions('ws');
    const t = task('t1', 'AB-1', 'c1', 'board-1');

    store.board = {
      id: 'board-1',
      name: 'My Board',
      workspace_id: 'ws',
      project_id: 'proj',
      created_at: '',
      updated_at: '',
      created_by: { id: 'u1', display_name: 'User', type: 'user' },
    };

    const items = ti.buildMenuItems({
      task: t,
      boardId: 'board-1',
      columns,
      allowDuplicate: true,
      onOpen: () => {},
    });

    const moveToBoard = items.find((it) => it.label === 'Move to board');
    expect(moveToBoard).toBeDefined();
  });
});

describe('useBoardsStore — fetchColumnsForBoard', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('fetchColumnsForBoard returns columns without clobbering boards.columns', async () => {
    const store = useBoardsStore();

    store.columns = [col('active-c1', 'Active Column')];

    GET.mockResolvedValueOnce({
      data: [col('other-c1', 'Other Column'), col('other-c2', 'Other Status')],
      error: undefined,
    });

    const fetched = await store.fetchColumnsForBoard('ws', 'other-board');

    expect(fetched.map((c) => c.name)).toContain('Other Column');
    expect(fetched.map((c) => c.name)).toContain('Other Status');

    expect(store.columns.map((c) => c.name)).toContain('Active Column');
    expect(store.columns.map((c) => c.name)).not.toContain('Other Column');
  });

  it('fetchColumnsForBoard caches results and does not re-fetch', async () => {
    const store = useBoardsStore();

    GET.mockResolvedValueOnce({
      data: [col('c1', 'Cached Col')],
      error: undefined,
    });

    const first = await store.fetchColumnsForBoard('ws', 'some-board');
    const second = await store.fetchColumnsForBoard('ws', 'some-board');

    expect(GET).toHaveBeenCalledTimes(1);
    expect(second).toEqual(first);
  });

  it('fetchColumnsForBoard returns [] on API error without clobbering active columns', async () => {
    const store = useBoardsStore();
    store.columns = [col('active-c1', 'Active Column')];

    GET.mockResolvedValueOnce({ data: undefined, error: { hint: 'not found' } });

    const fetched = await store.fetchColumnsForBoard('ws', 'bad-board');

    expect(fetched).toEqual([]);
    expect(store.columns.map((c) => c.name)).toContain('Active Column');
  });
});
