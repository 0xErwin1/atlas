import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET, POST, DELETE, PATCH } = vi.hoisted(() => ({
  GET: vi.fn(),
  POST: vi.fn(),
  DELETE: vi.fn(),
  PATCH: vi.fn(),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET, POST, DELETE, PATCH },
}));

import { useTaskDetailStore } from '@/stores/taskDetail';

const actor = (id: string, type: string, name: string) => ({
  id,
  type,
  display_name: name,
});

const assignee = (id: string, type: string, name: string) => ({
  assignee: actor(id, type, name),
  assigned_by: actor('admin', 'user', 'Admin'),
  assigned_at: '2026-01-01T00:00:00Z',
});

const checklistItem = (id: string, title: string, checked: boolean) => ({
  id,
  task_id: 't1',
  title,
  checked,
  position_key: 'a',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
});

const subtaskSummary = (id: string, readableId: string, title: string) => ({
  id,
  readable_id: readableId,
  board_id: 'board-1',
  column_id: 'col-1',
  board_name: 'Board',
  column_name: 'Todo',
  title,
  estimate: null,
  labels: [],
  assignees: [],
  updated_at: '2026-01-01T00:00:00Z',
});

const reference = (id: string, kind: string) => ({
  id,
  kind,
  created_at: '2026-01-01T00:00:00Z',
  created_by: actor('u1', 'user', 'User'),
  target_resolved: true,
  target_readable_id: 'ATL-9',
});

const activityEntry = (id: string, kind: string, actorType: string, name: string) => ({
  id,
  kind,
  actor: actor(`a-${id}`, actorType, name),
  payload: {},
  created_at: '2026-01-01T00:00:00Z',
  task_id: `t-${id}`,
  task_readable_id: `ATL-${id}`,
});

const comment = (
  id: string,
  body: string,
  actorType: string,
  name: string,
  updatedAt = '2026-01-01T00:00:00Z',
) => ({
  id,
  task_id: 't1',
  body,
  author: actor(`a-${id}`, actorType, name),
  created_at: '2026-01-01T00:00:00Z',
  updated_at: updatedAt,
});

describe('useTaskDetailStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('loadAll populates assignees, references, subtasks, checklist, activity and comments', async () => {
    GET.mockImplementation((path: string) => {
      if (path.endsWith('/assignees')) {
        return Promise.resolve({ data: [assignee('u9', 'user', 'Jordan')], error: undefined });
      }
      if (path.endsWith('/references')) {
        return Promise.resolve({ data: [reference('r1', 'relates')], error: undefined });
      }
      if (path.endsWith('/subtasks')) {
        return Promise.resolve({ data: [subtaskSummary('s1', 'ATL-2', 'Child')], error: undefined });
      }
      if (path.endsWith('/checklist')) {
        return Promise.resolve({ data: [checklistItem('c1', 'Review code', false)], error: undefined });
      }
      if (path.endsWith('/activity')) {
        return Promise.resolve({
          data: { items: [activityEntry('e1', 'created', 'user', 'Jordan')], has_more: false },
          error: undefined,
        });
      }
      if (path.endsWith('/comments')) {
        return Promise.resolve({
          data: {
            items: [comment('cm1', 'First comment', 'user', 'Jordan')],
            has_more: true,
            next_cursor: 'cm1',
          },
          error: undefined,
        });
      }
      return Promise.resolve({ data: undefined, error: { hint: 'unexpected' } });
    });

    const store = useTaskDetailStore();
    await store.loadAll('ws', 'ATL-1');

    expect(store.assignees).toHaveLength(1);
    expect(store.assignees[0]?.assignee.display_name).toBe('Jordan');
    expect(store.references).toHaveLength(1);
    expect(store.subtasks).toHaveLength(1);
    expect(store.subtasks[0]?.readable_id).toBe('ATL-2');
    expect(store.checklist).toHaveLength(1);
    expect(store.checklist[0]?.title).toBe('Review code');
    expect(store.activity).toHaveLength(1);
    expect(store.comments).toHaveLength(1);
    expect(store.comments[0]?.body).toBe('First comment');
    expect(store.commentsHasMore).toBe(true);
    expect(store.commentsCursor).toBe('cm1');
  });

  it('addChecklistItem appends the created item to checklist on success', async () => {
    const store = useTaskDetailStore();

    POST.mockResolvedValueOnce({
      data: checklistItem('c9', 'Write tests', false),
      error: undefined,
    });

    const ok = await store.addChecklistItem('ws', 'ATL-1', 'Write tests');

    expect(ok).toBe(true);
    expect(store.checklist).toHaveLength(1);
    expect(store.checklist[0]?.title).toBe('Write tests');
  });

  it('addChecklistItem surfaces the hint and returns false on error', async () => {
    const store = useTaskDetailStore();

    POST.mockResolvedValueOnce({ data: undefined, error: { hint: 'Not found' } });

    const ok = await store.addChecklistItem('ws', 'ATL-1', 'Step');

    expect(ok).toBe(false);
    expect(store.checklist).toHaveLength(0);
    expect(store.error).toBe('Not found');
  });

  it('addSubtask appends the created child as a summary row', async () => {
    const store = useTaskDetailStore();

    POST.mockResolvedValueOnce({
      data: {
        id: 's2',
        readable_id: 'ATL-3',
        column_id: 'col-1',
        title: 'New child',
        estimate: null,
        labels: [],
        updated_at: '2026-01-01T00:00:00Z',
      },
      error: undefined,
    });

    const ok = await store.addSubtask('ws', 'ATL-1', 'New child');

    expect(ok).toBe(true);
    expect(store.subtasks).toHaveLength(1);
    expect(store.subtasks[0]?.readable_id).toBe('ATL-3');
  });

  it('promoteSubtask removes the child on success', async () => {
    const store = useTaskDetailStore();
    store._setForTest({ subtasks: [subtaskSummary('s1', 'ATL-2', 'Child')] });

    POST.mockResolvedValueOnce({ data: undefined, error: undefined });

    const ok = await store.promoteSubtask('ws', 'ATL-2');

    expect(ok).toBe(true);
    expect(store.subtasks).toHaveLength(0);
  });

  it('addAssignee optimistically appends, then reconciles with the server DTO', async () => {
    const store = useTaskDetailStore();
    store._setForTest({ assignees: [assignee('u1', 'user', 'Ann')] });

    const created = assignee('agent-1', 'api_key', 'Claude');
    POST.mockResolvedValueOnce({ data: created, error: undefined });

    const ok = await store.addAssignee('ws', 'ATL-1', { assignee_id: 'agent-1', assignee_type: 'api_key' });

    expect(ok).toBe(true);
    expect(store.assignees).toHaveLength(2);
    expect(store.assignees.some((a) => a.assignee.id === 'agent-1')).toBe(true);
  });

  it('addAssignee rolls back and surfaces the hint on error', async () => {
    const store = useTaskDetailStore();
    store._setForTest({ assignees: [assignee('u1', 'user', 'Ann')] });

    POST.mockResolvedValueOnce({
      data: undefined,
      error: { hint: 'Already assigned', status: 409 },
    });

    const ok = await store.addAssignee('ws', 'ATL-1', { assignee_id: 'u1', assignee_type: 'user' });

    expect(ok).toBe(false);
    expect(store.error).toBe('Already assigned');
    expect(store.assignees).toHaveLength(1);
  });

  it('addAssignee surfaces the 422 detail when assigning a deactivated user', async () => {
    const store = useTaskDetailStore();
    store._setForTest({ assignees: [assignee('u1', 'user', 'Ann')] });

    POST.mockResolvedValueOnce({
      data: undefined,
      error: {
        status: 422,
        detail: 'Cannot assign a deactivated user; re-enable the account first.',
      },
    });

    const ok = await store.addAssignee('ws', 'ATL-1', { assignee_id: 'u2', assignee_type: 'user' });

    expect(ok).toBe(false);
    expect(store.error).toBe('Cannot assign a deactivated user; re-enable the account first.');
    expect(store.assignees).toHaveLength(1);
  });

  it('removeAssignee optimistically removes, rolls back on error', async () => {
    const store = useTaskDetailStore();
    store._setForTest({
      assignees: [assignee('u1', 'user', 'Ann'), assignee('agent-1', 'api_key', 'Claude')],
    });

    DELETE.mockResolvedValueOnce({ data: undefined, error: { hint: 'No permission' } });

    const ok = await store.removeAssignee('ws', 'ATL-1', 'api_key', 'agent-1');

    expect(ok).toBe(false);
    expect(store.error).toBe('No permission');
    expect(store.assignees).toHaveLength(2);
  });

  it('removeAssignee builds the api_key:{id} ref and removes on success', async () => {
    const store = useTaskDetailStore();
    store._setForTest({
      assignees: [assignee('u1', 'user', 'Ann'), assignee('agent-1', 'api_key', 'Claude')],
    });

    DELETE.mockResolvedValueOnce({ data: undefined, error: undefined });

    const ok = await store.removeAssignee('ws', 'ATL-1', 'api_key', 'agent-1');

    expect(ok).toBe(true);
    expect(store.assignees).toHaveLength(1);
    expect(store.assignees[0]?.assignee.id).toBe('u1');

    const [, opts] = DELETE.mock.calls[0] as [string, { params: { path: { assignee_ref: string } } }];
    expect(opts.params.path.assignee_ref).toBe('api_key:agent-1');
  });

  it('toggleChecklistItem flips checked optimistically and PATCHes', async () => {
    const store = useTaskDetailStore();
    store._setForTest({ checklist: [checklistItem('c1', 'Step', false)] });

    PATCH.mockResolvedValueOnce({
      data: checklistItem('c1', 'Step', true),
      error: undefined,
    });

    const ok = await store.toggleChecklistItem('ws', 'ATL-1', 'c1');

    expect(ok).toBe(true);
    expect(store.checklist[0]?.checked).toBe(true);

    const [, opts] = PATCH.mock.calls[0] as [string, { body: { checked?: boolean | null } }];
    expect(opts.body.checked).toBe(true);
  });

  it('toggleChecklistItem rolls back the optimistic flip on error', async () => {
    const store = useTaskDetailStore();
    store._setForTest({ checklist: [checklistItem('c1', 'Step', false)] });

    PATCH.mockResolvedValueOnce({ data: undefined, error: { hint: 'Failed' } });

    const ok = await store.toggleChecklistItem('ws', 'ATL-1', 'c1');

    expect(ok).toBe(false);
    expect(store.checklist[0]?.checked).toBe(false);
    expect(store.error).toBe('Failed');
  });

  it('updateChecklistItem edits the title optimistically and PATCHes', async () => {
    const store = useTaskDetailStore();
    store._setForTest({ checklist: [checklistItem('c1', 'Step', false)] });

    PATCH.mockResolvedValueOnce({
      data: { ...checklistItem('c1', 'Step edited', false) },
      error: undefined,
    });

    const ok = await store.updateChecklistItem('ws', 'ATL-1', 'c1', '  Step edited  ');

    expect(ok).toBe(true);
    expect(store.checklist[0]?.title).toBe('Step edited');

    const [, opts] = PATCH.mock.calls[0] as [string, { body: { title?: string | null } }];
    expect(opts.body.title).toBe('Step edited');
  });

  it('updateChecklistItem rolls back the optimistic title on error', async () => {
    const store = useTaskDetailStore();
    store._setForTest({ checklist: [checklistItem('c1', 'Step', false)] });

    PATCH.mockResolvedValueOnce({ data: undefined, error: { hint: 'Failed' } });

    const ok = await store.updateChecklistItem('ws', 'ATL-1', 'c1', 'Step edited');

    expect(ok).toBe(false);
    expect(store.checklist[0]?.title).toBe('Step');
    expect(store.error).toBe('Failed');
  });

  it('updateChecklistItem is a no-op that never PATCHes when the title is unchanged', async () => {
    const store = useTaskDetailStore();
    store._setForTest({ checklist: [checklistItem('c1', 'Step', false)] });

    const ok = await store.updateChecklistItem('ws', 'ATL-1', 'c1', '  Step  ');

    expect(ok).toBe(false);
    expect(PATCH).not.toHaveBeenCalled();
    expect(store.error).toBeNull();
  });

  it('removeChecklistItem deletes the item on success', async () => {
    const store = useTaskDetailStore();
    store._setForTest({
      checklist: [checklistItem('c1', 'Step', false), checklistItem('c2', 'Other', false)],
    });

    DELETE.mockResolvedValueOnce({ data: undefined, error: undefined });

    const ok = await store.removeChecklistItem('ws', 'ATL-1', 'c1');

    expect(ok).toBe(true);
    expect(store.checklist.map((i) => i.id)).toEqual(['c2']);
  });

  it('removeChecklistItem keeps the item and sets error on failure', async () => {
    const store = useTaskDetailStore();
    store._setForTest({ checklist: [checklistItem('c1', 'Step', false)] });

    DELETE.mockResolvedValueOnce({ data: undefined, error: { hint: 'No permission' } });

    const ok = await store.removeChecklistItem('ws', 'ATL-1', 'c1');

    expect(ok).toBe(false);
    expect(store.checklist).toHaveLength(1);
    expect(store.error).toBe('No permission');
  });

  it('promoteChecklistItem POSTs board+column and marks the item promoted on success', async () => {
    const store = useTaskDetailStore();
    store._setForTest({ checklist: [checklistItem('c1', 'Step', false)] });

    const promoted = {
      checklist_item: { ...checklistItem('c1', 'Step', false), promoted_readable_id: 'ATL-77' },
      task: { readable_id: 'ATL-77' },
      parent_reference: null,
    };
    POST.mockResolvedValueOnce({ data: promoted, error: undefined });

    const result = await store.promoteChecklistItem('ws', 'ATL-1', 'c1', 'board-1', 'col-1');

    expect(result.ok).toBe(true);
    expect(result.readableId).toBe('ATL-77');
    expect(store.checklist[0]?.promoted_readable_id).toBe('ATL-77');

    const [, opts] = POST.mock.calls[0] as [string, { body: { board_id: string; column_id: string } }];
    expect(opts.body.board_id).toBe('board-1');
    expect(opts.body.column_id).toBe('col-1');
  });

  it('promoteChecklistItem surfaces the hint on error and leaves the item unpromoted', async () => {
    const store = useTaskDetailStore();
    store._setForTest({ checklist: [checklistItem('c1', 'Step', false)] });

    POST.mockResolvedValueOnce({ data: undefined, error: { hint: 'Cannot promote' } });

    const result = await store.promoteChecklistItem('ws', 'ATL-1', 'c1', 'board-1', 'col-1');

    expect(result.ok).toBe(false);
    expect(store.error).toBe('Cannot promote');
    expect(store.checklist[0]?.promoted_readable_id).toBeFalsy();
  });

  it('clear resets all collections', async () => {
    const store = useTaskDetailStore();
    store._setForTest({
      assignees: [assignee('u1', 'user', 'Ann')],
      checklist: [checklistItem('c1', 'Step', false)],
      references: [reference('r1', 'relates')],
      activity: [activityEntry('e1', 'created', 'user', 'Ann')],
      comments: [comment('cm1', 'Hello', 'user', 'Ann')],
    });

    store.clear();

    expect(store.assignees).toHaveLength(0);
    expect(store.checklist).toHaveLength(0);
    expect(store.references).toHaveLength(0);
    expect(store.activity).toHaveLength(0);
    expect(store.comments).toHaveLength(0);
    expect(store.commentsCursor).toBeNull();
    expect(store.commentsHasMore).toBe(false);
  });

  it('loadMoreComments appends the next page using the stored cursor, oldest-first', async () => {
    const store = useTaskDetailStore();
    store._setForTest({
      comments: [comment('cm1', 'First', 'user', 'Ann')],
      commentsCursor: 'cm1',
      commentsHasMore: true,
    });

    GET.mockResolvedValueOnce({
      data: { items: [comment('cm2', 'Second', 'user', 'Ann')], has_more: false, next_cursor: null },
      error: undefined,
    });

    await store.loadMoreComments('ws', 'ATL-1');

    expect(store.comments.map((c) => c.id)).toEqual(['cm1', 'cm2']);
    expect(store.commentsHasMore).toBe(false);
    expect(store.commentsCursor).toBeNull();

    const [, opts] = GET.mock.calls[0] as [string, { params: { query?: { cursor?: string } } }];
    expect(opts.params.query?.cursor).toBe('cm1');
  });

  it('loadMoreComments is a no-op when there is no further page', async () => {
    const store = useTaskDetailStore();
    store._setForTest({
      comments: [comment('cm1', 'First', 'user', 'Ann')],
      commentsCursor: null,
      commentsHasMore: false,
    });

    await store.loadMoreComments('ws', 'ATL-1');

    expect(GET).not.toHaveBeenCalled();
    expect(store.comments).toHaveLength(1);
  });

  it('addComment appends the created comment at the end on success when fully paged', async () => {
    const store = useTaskDetailStore();
    store._setForTest({ comments: [comment('cm1', 'First', 'user', 'Ann')], commentsHasMore: false });

    POST.mockResolvedValueOnce({
      data: comment('cm2', 'Second', 'user', 'Ann'),
      error: undefined,
    });

    const ok = await store.addComment('ws', 'ATL-1', 'Second');

    expect(ok).toBe(true);
    expect(store.comments.map((c) => c.id)).toEqual(['cm1', 'cm2']);

    const [, opts] = POST.mock.calls[0] as [string, { body: { body: string } }];
    expect(opts.body.body).toBe('Second');
  });

  it('addComment does not append while earlier pages remain unloaded', async () => {
    const store = useTaskDetailStore();
    store._setForTest({
      comments: [comment('cm1', 'First', 'user', 'Ann')],
      commentsCursor: 'cm1',
      commentsHasMore: true,
    });

    POST.mockResolvedValueOnce({
      data: comment('cm2', 'Second', 'user', 'Ann'),
      error: undefined,
    });

    const ok = await store.addComment('ws', 'ATL-1', 'Second');

    expect(ok).toBe(true);
    expect(store.comments.map((c) => c.id)).toEqual(['cm1']);
  });

  it('addComment surfaces the hint and returns false on error', async () => {
    const store = useTaskDetailStore();

    POST.mockResolvedValueOnce({ data: undefined, error: { hint: 'Comment too long' } });

    const ok = await store.addComment('ws', 'ATL-1', 'x'.repeat(10_001));

    expect(ok).toBe(false);
    expect(store.comments).toHaveLength(0);
    expect(store.error).toBe('Comment too long');
  });

  it('editComment swaps the updated comment in place on success', async () => {
    const store = useTaskDetailStore();
    store._setForTest({
      comments: [comment('cm1', 'First', 'user', 'Ann'), comment('cm2', 'Second', 'user', 'Ann')],
    });

    PATCH.mockResolvedValueOnce({
      data: comment('cm1', 'First edited', 'user', 'Ann', '2026-02-02T00:00:00Z'),
      error: undefined,
    });

    const ok = await store.editComment('ws', 'ATL-1', 'cm1', 'First edited');

    expect(ok).toBe(true);
    expect(store.comments.map((c) => c.id)).toEqual(['cm1', 'cm2']);
    expect(store.comments[0]?.body).toBe('First edited');
    expect(store.comments[0]?.updated_at).toBe('2026-02-02T00:00:00Z');

    const [, opts] = PATCH.mock.calls[0] as [
      string,
      { params: { path: { comment_id: string } }; body: { body: string } },
    ];
    expect(opts.params.path.comment_id).toBe('cm1');
    expect(opts.body.body).toBe('First edited');
  });

  it('editComment surfaces the hint and leaves the comment unchanged on error', async () => {
    const store = useTaskDetailStore();
    store._setForTest({ comments: [comment('cm1', 'First', 'user', 'Ann')] });

    PATCH.mockResolvedValueOnce({ data: undefined, error: { hint: 'No permission' } });

    const ok = await store.editComment('ws', 'ATL-1', 'cm1', 'Nope');

    expect(ok).toBe(false);
    expect(store.error).toBe('No permission');
    expect(store.comments[0]?.body).toBe('First');
  });

  it('removeComment optimistically removes, rolls back on error', async () => {
    const store = useTaskDetailStore();
    store._setForTest({
      comments: [comment('cm1', 'First', 'user', 'Ann'), comment('cm2', 'Second', 'user', 'Ann')],
    });

    DELETE.mockResolvedValueOnce({ data: undefined, error: { hint: 'No permission' } });

    const ok = await store.removeComment('ws', 'ATL-1', 'cm2');

    expect(ok).toBe(false);
    expect(store.error).toBe('No permission');
    expect(store.comments).toHaveLength(2);
  });

  it('removeComment deletes on success', async () => {
    const store = useTaskDetailStore();
    store._setForTest({
      comments: [comment('cm1', 'First', 'user', 'Ann'), comment('cm2', 'Second', 'user', 'Ann')],
    });

    DELETE.mockResolvedValueOnce({ data: undefined, error: undefined });

    const ok = await store.removeComment('ws', 'ATL-1', 'cm1');

    expect(ok).toBe(true);
    expect(store.comments.map((c) => c.id)).toEqual(['cm2']);

    const [, opts] = DELETE.mock.calls[0] as [string, { params: { path: { comment_id: string } } }];
    expect(opts.params.path.comment_id).toBe('cm1');
  });
});
