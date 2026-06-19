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
});

describe('useTaskDetailStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('loadAll populates assignees, references, checklist and activity', async () => {
    GET.mockImplementation((path: string) => {
      if (path.endsWith('/assignees')) {
        return Promise.resolve({ data: [assignee('u9', 'user', 'Jordan')], error: undefined });
      }
      if (path.endsWith('/references')) {
        return Promise.resolve({ data: [reference('r1', 'relates')], error: undefined });
      }
      if (path.endsWith('/checklist')) {
        return Promise.resolve({ data: [checklistItem('c1', 'Step', false)], error: undefined });
      }
      if (path.endsWith('/activity')) {
        return Promise.resolve({
          data: { items: [activityEntry('e1', 'created', 'user', 'Jordan')], has_more: false },
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
    expect(store.checklist).toHaveLength(1);
    expect(store.activity).toHaveLength(1);
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
    });

    store.clear();

    expect(store.assignees).toHaveLength(0);
    expect(store.checklist).toHaveLength(0);
    expect(store.references).toHaveLength(0);
    expect(store.activity).toHaveLength(0);
  });
});
