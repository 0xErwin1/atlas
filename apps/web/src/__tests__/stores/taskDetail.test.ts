import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET, POST, DELETE, PATCH } = vi.hoisted(() => ({
  GET: vi.fn(),
  POST: vi.fn(),
  DELETE: vi.fn(),
  PATCH: vi.fn(),
}));

const { cacheHydrate, cacheRevalidate, cacheActivate, cacheDeactivate, cachePurgeTags, cacheIsAvailable } =
  vi.hoisted(() => ({
    cacheHydrate: vi.fn(),
    cacheRevalidate: vi.fn(),
    cacheActivate: vi.fn(),
    cacheDeactivate: vi.fn(),
    cachePurgeTags: vi.fn(),
    cacheIsAvailable: vi.fn(() => true),
  }));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET, POST, DELETE, PATCH },
}));

vi.mock('@/cache/cacheRuntime', () => ({
  getResourceCachePrincipal: () => 'user:018f0000-0000-7000-8000-000000000001',
  resourceCacheEpoch: { __v_isRef: true, value: 0 },
  invalidateTaskCache: cachePurgeTags,
  resourceCache: {
    isAvailable: cacheIsAvailable,
    hydrate: cacheHydrate,
    revalidate: cacheRevalidate,
    activate: cacheActivate,
    deactivate: cacheDeactivate,
  },
}));

import { type ReferenceDto, useTaskDetailStore } from '@/stores/taskDetail';
import { useTasksStore } from '@/stores/tasks';

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
  subtask_count: 0,
  updated_at: '2026-01-01T00:00:00Z',
});

const reference = (id: string, kind: string): ReferenceDto => ({
  id,
  origins: ['manual'],
  wikilink_reference_id: null,
  manual_reference_id: id,
  manual_kind: kind,
  manual_created_at: '2026-01-01T00:00:00Z',
  manual_created_by: actor('u1', 'user', 'User'),
  target_task_id: 'task-9',
  target_document_id: null,
  target_title: null,
  target_resolved: true,
  target_readable_id: 'ATL-9',
});

const mergedReference = (manualId: string, wikilinkId: string): ReferenceDto => ({
  ...reference(manualId, 'docs'),
  origins: ['manual', 'wikilink'],
  wikilink_reference_id: wikilinkId,
  target_task_id: null,
  target_document_id: 'doc-9',
  target_readable_id: null,
  target_title: 'Linked document',
});

const wikilinkReference = (id: string, title: string): ReferenceDto => ({
  id,
  origins: ['wikilink'],
  wikilink_reference_id: id,
  manual_reference_id: null,
  manual_kind: null,
  manual_created_at: null,
  manual_created_by: null,
  target_task_id: null,
  target_document_id: 'doc-9',
  target_readable_id: null,
  target_title: title,
  target_resolved: true,
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

const collectionResponse = (index: number, name: string) => {
  switch (index) {
    case 0:
      return { data: [assignee(`u-${name}`, 'user', name)], error: undefined };
    case 1:
      return { data: [reference(`r-${name}`, 'relates')], error: undefined };
    case 2:
      return {
        data: { items: [{ source_readable_id: `ATL-${name}`, source_title: name, kind: 'relates' }] },
        error: undefined,
      };
    case 3:
      return { data: [subtaskSummary(`s-${name}`, `ATL-${name}`, name)], error: undefined };
    case 4:
      return { data: [checklistItem(`c-${name}`, name, false)], error: undefined };
    case 5:
      return { data: { items: [activityEntry(`a-${name}`, 'created', 'user', name)] }, error: undefined };
    case 6:
      return { data: [{ id: `at-${name}`, file_name: `${name}.txt` }], error: undefined };
    default:
      return {
        data: { items: [comment(`cm-${name}`, name, 'user', name)], has_more: false, next_cursor: null },
        error: undefined,
      };
  }
};

const collectionIndex = (path: string): number =>
  [
    'assignees',
    'references',
    'backlinks',
    'subtasks',
    'checklist',
    'activity',
    'attachments',
    'comments',
  ].findIndex((suffix) => path.endsWith(`/${suffix}`));

function deferred<T>() {
  let resolve: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });

  return { promise, resolve: (value: T) => resolve(value) };
}

describe('useTaskDetailStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    GET.mockReset();
    POST.mockReset();
    DELETE.mockReset();
    PATCH.mockReset();
    cacheHydrate.mockReset();
    cacheRevalidate.mockReset();
    cacheActivate.mockReset();
    cacheDeactivate.mockReset();
    cachePurgeTags.mockReset();
    cacheIsAvailable.mockReset();
    cacheIsAvailable.mockReturnValue(true);
  });

  it('loadAll populates assignees, references, subtasks, checklist, activity and comments', async () => {
    GET.mockImplementation((path: string) => {
      if (path.endsWith('/assignees')) {
        return Promise.resolve({ data: [assignee('u9', 'user', 'Jordan')], error: undefined });
      }
      if (path.endsWith('/references')) {
        return Promise.resolve({ data: [reference('r1', 'relates')], error: undefined });
      }
      if (path.endsWith('/backlinks')) {
        return Promise.resolve({
          data: {
            items: [
              {
                source_task_id: 't5',
                source_readable_id: 'ATL-5',
                source_title: 'Blocks this',
                kind: 'blocks',
              },
            ],
            has_more: false,
            next_cursor: null,
          },
          error: undefined,
        });
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
    expect(store.backlinks).toHaveLength(1);
    expect(store.backlinks[0]?.source_readable_id).toBe('ATL-5');
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

  it('hydrates each current task collection independently before failed refreshes retain cached data', async () => {
    cacheHydrate.mockImplementation(async (request) => {
      if (request.key.includes('assignees')) {
        request.publish([assignee('u-cache', 'user', 'Cached user')]);
      }
      return null;
    });
    cacheRevalidate.mockRejectedValue(new Error('Network unavailable'));

    const store = useTaskDetailStore();
    await store.loadAll(
      'ws',
      'ATL-1',
      '018f0000-0000-7000-8000-000000000002',
      '018f0000-0000-7000-8000-000000000003',
    );

    expect(store.assignees[0]?.assignee.display_name).toBe('Cached user');
    expect(store.collectionStatus.assignees).toBe('error');
    expect(store.collectionStatus.references).toBe('error');
    expect(cacheHydrate).toHaveBeenCalledTimes(8);
    expect(cacheActivate).toHaveBeenCalledTimes(8);
    expect(cacheHydrate.mock.calls[0]?.[0].tags).toContain('task-uuid:018f0000-0000-7000-8000-000000000003');
  });

  it.each([
    403, 404,
  ])('retracts every collection and the primary task after an initial %i detail denial', async (status) => {
    cacheHydrate.mockResolvedValue(null);
    cacheRevalidate.mockImplementation(async (request) => request.publish(await request.load()));
    GET.mockImplementation((path: string) => {
      if (path.endsWith('/references')) {
        return Promise.resolve({ data: undefined, error: { status, hint: 'Denied' } });
      }
      return Promise.resolve(collectionResponse(collectionIndex(path), 'Visible'));
    });

    const primary = useTasksStore();
    GET.mockResolvedValueOnce({
      data: {
        id: '018f0000-0000-7000-8000-000000000003',
        readable_id: 'ATL-1',
        workspace_id: 'workspace',
        board_id: 'board-1',
        column_id: 'column-1',
        project_id: 'project-1',
        title: 'Visible task',
        description: '',
        created_at: '2026-01-01T00:00:00Z',
        updated_at: '2026-01-01T00:00:00Z',
        created_by: actor('u1', 'user', 'User'),
      },
      error: undefined,
    });
    await primary.loadTask('ws', 'ATL-1');
    const store = useTaskDetailStore();
    await store.loadAll(
      'ws',
      'ATL-1',
      '018f0000-0000-7000-8000-000000000002',
      '018f0000-0000-7000-8000-000000000003',
    );

    expect(store.assignees).toEqual([]);
    expect(store.references).toEqual([]);
    expect(store.comments).toEqual([]);
    expect(primary.openTask).toBeNull();
    expect(cachePurgeTags).toHaveBeenCalledWith(
      '018f0000-0000-7000-8000-000000000002',
      'ATL-1',
      undefined,
      '018f0000-0000-7000-8000-000000000003',
    );
  });

  it.each([403, 404])('retracts all detail state after an online-only initial %i denial', async (status) => {
    cacheIsAvailable.mockReturnValue(false);
    GET.mockImplementation((path: string) => {
      if (path.endsWith('/references')) {
        return Promise.resolve({ data: undefined, error: { status, hint: 'Denied' } });
      }
      return Promise.resolve(collectionResponse(collectionIndex(path), 'Visible'));
    });

    const primary = useTasksStore();
    GET.mockResolvedValueOnce({
      data: {
        id: '018f0000-0000-7000-8000-000000000003',
        readable_id: 'ATL-1',
        workspace_id: 'workspace',
        board_id: 'board-1',
        column_id: 'column-1',
        project_id: 'project-1',
        title: 'Visible task',
        description: '',
        created_at: '2026-01-01T00:00:00Z',
        updated_at: '2026-01-01T00:00:00Z',
        created_by: actor('u1', 'user', 'User'),
      },
      error: undefined,
    });
    await primary.loadTask('ws', 'ATL-1');

    const store = useTaskDetailStore();
    await store.loadAll(
      'ws',
      'ATL-1',
      '018f0000-0000-7000-8000-000000000002',
      '018f0000-0000-7000-8000-000000000003',
    );

    expect(store.assignees).toEqual([]);
    expect(store.references).toEqual([]);
    expect(store.comments).toEqual([]);
    expect(primary.openTask).toBeNull();
    expect(cachePurgeTags).toHaveBeenCalledWith(
      '018f0000-0000-7000-8000-000000000002',
      'ATL-1',
      undefined,
      '018f0000-0000-7000-8000-000000000003',
    );
  });

  it('keeps detail collections online-only when the authoritative task UUID is unavailable', async () => {
    GET.mockImplementation((path: string) =>
      Promise.resolve(collectionResponse(collectionIndex(path), 'Online')),
    );

    const store = useTaskDetailStore();
    await store.loadAll('ws', 'ATL-1', '018f0000-0000-7000-8000-000000000002');

    expect(store.assignees[0]?.assignee.display_name).toBe('Online');
    expect(cacheHydrate).not.toHaveBeenCalled();
    expect(cacheActivate).not.toHaveBeenCalled();
  });

  it('caches each comments page under its exact cursor and preserves page order', async () => {
    cacheHydrate.mockImplementation(async () => null);
    cacheRevalidate.mockImplementation(async (request) => request.publish(await request.load()));
    GET.mockImplementation((path: string, options?: { params?: { query?: { cursor?: string } } }) => {
      if (path.endsWith('/comments') && options?.params?.query?.cursor === 'page-1') {
        return Promise.resolve({
          data: { items: [comment('cm2', 'Second', 'user', 'Jordan')], has_more: false, next_cursor: null },
          error: undefined,
        });
      }
      return Promise.resolve(collectionResponse(collectionIndex(path), 'First'));
    });

    const store = useTaskDetailStore();
    await store.loadAll(
      'ws',
      'ATL-1',
      '018f0000-0000-7000-8000-000000000002',
      '018f0000-0000-7000-8000-000000000003',
    );
    store._setForTest({
      comments: [comment('cm1', 'First', 'user', 'Jordan')],
      commentsCursor: 'page-1',
      commentsHasMore: true,
    });

    await store.loadMoreComments('ws', 'ATL-1');

    expect(store.comments.map((item) => item.body)).toEqual(['First', 'Second']);
    expect(cacheHydrate.mock.calls.at(-1)?.[0].key).toContain('comments:page-1');
    expect(cacheActivate.mock.calls.at(-1)?.[0].key).toContain('comments:page-1');
  });

  it('resets every detail collection for a workspace-only target change and rejects late results', async () => {
    const pending: Array<{ resolve: (value: unknown) => void }> = [];
    GET.mockImplementation(
      () =>
        new Promise((resolve) => {
          pending.push({ resolve });
        }),
    );

    const store = useTaskDetailStore();
    store._setForTest({
      assignees: [assignee('u1', 'user', 'Ann')],
      references: [reference('r1', 'relates')],
      backlinks: [
        { source_task_id: 't2', source_readable_id: 'ATL-2', source_title: 'Old', kind: 'relates' },
      ],
      subtasks: [subtaskSummary('s1', 'ATL-2', 'Old child')],
      checklist: [checklistItem('c1', 'Old item', false)],
      activity: [activityEntry('a1', 'created', 'user', 'Ann')],
      attachments: [
        {
          id: 'at1',
          file_name: 'old.txt',
          content_type: 'text/plain',
          created_at: '2026-01-01T00:00:00Z',
          created_by: actor('u1', 'user', 'User'),
          size_bytes: 1,
        },
      ],
      comments: [comment('cm1', 'Old comment', 'user', 'Ann')],
      commentsCursor: 'cm1',
      commentsHasMore: true,
    });

    const prior = store.loadAll('workspace-a', 'ATL-1');
    const current = store.loadAll('workspace-b', 'ATL-1');

    expect(store.assignees).toEqual([]);
    expect(store.references).toEqual([]);
    expect(store.backlinks).toEqual([]);
    expect(store.subtasks).toEqual([]);
    expect(store.checklist).toEqual([]);
    expect(store.activity).toEqual([]);
    expect(store.attachments).toEqual([]);
    expect(store.comments).toEqual([]);
    expect(store.commentsCursor).toBeNull();
    expect(store.commentsHasMore).toBe(false);
    expect(Object.values(store.collectionStatus)).toEqual(Array(8).fill('pending'));

    for (const [index, request] of pending.slice(0, 8).entries()) {
      request.resolve(collectionResponse(index, 'Old workspace'));
    }
    await prior;

    expect(store.assignees).toEqual([]);
    expect(store.references).toEqual([]);
    expect(store.backlinks).toEqual([]);
    expect(store.subtasks).toEqual([]);
    expect(store.checklist).toEqual([]);
    expect(store.activity).toEqual([]);
    expect(store.attachments).toEqual([]);
    expect(store.comments).toEqual([]);

    for (const [index, request] of pending.slice(8).entries()) {
      request.resolve(collectionResponse(index, 'Current workspace'));
    }
    await current;

    expect(store.assignees[0]?.assignee.display_name).toBe('Current workspace');
    expect(store.references[0]?.id).toBe('r-Current workspace');
    expect(store.backlinks[0]?.source_title).toBe('Current workspace');
    expect(store.subtasks[0]?.title).toBe('Current workspace');
    expect(store.checklist[0]?.title).toBe('Current workspace');
    expect(store.activity[0]?.actor.display_name).toBe('Current workspace');
    expect(store.attachments[0]?.file_name).toBe('Current workspace.txt');
    expect(store.comments[0]?.body).toBe('Current workspace');
    expect(Object.values(store.collectionStatus)).toEqual(Array(8).fill('ready'));
  });

  it('settles the eight detail collections independently and preserves ready content for a same-target refresh', async () => {
    const store = useTaskDetailStore();
    GET.mockImplementation((path: string) =>
      Promise.resolve(collectionResponse(collectionIndex(path), 'Ready')),
    );

    await store.loadAll('ws', 'ATL-1');
    const retained = store.assignees[0];

    GET.mockImplementation((path: string) => {
      if (path.endsWith('/references')) {
        return Promise.resolve({ data: undefined, error: { hint: 'References unavailable' } });
      }
      return Promise.resolve(collectionResponse(collectionIndex(path), 'Refreshed'));
    });

    const refresh = store.loadAll('ws', 'ATL-1');

    expect(store.assignees[0]).toBe(retained);
    expect(Object.values(store.collectionStatus)).toEqual(Array(8).fill('pending'));

    await refresh;

    expect(store.collectionStatus.assignees).toBe('ready');
    expect(store.collectionStatus.references).toBe('error');
    expect(store.collectionErrors.references).toBe('References unavailable');
    expect(store.collectionStatus.backlinks).toBe('ready');
    expect(store.collectionStatus.subtasks).toBe('ready');
    expect(store.collectionStatus.checklist).toBe('ready');
    expect(store.collectionStatus.activity).toBe('ready');
    expect(store.collectionStatus.attachments).toBe('ready');
    expect(store.collectionStatus.comments).toBe('ready');
  });

  it('keeps untouched collections pending while each current-target collection settles', async () => {
    const requests: Array<ReturnType<typeof deferred<unknown>>> = [];
    GET.mockImplementation(() => {
      const request = deferred<unknown>();
      requests.push(request);
      return request.promise;
    });

    const store = useTaskDetailStore();
    const load = store.loadAll('ws', 'ATL-1');

    requests[0]?.resolve(collectionResponse(0, 'Ann'));
    await Promise.resolve();

    expect(store.collectionStatus.assignees).toBe('ready');
    expect(store.assignees[0]?.assignee.display_name).toBe('Ann');
    expect(store.collectionStatus.references).toBe('pending');
    expect(store.collectionStatus.backlinks).toBe('pending');
    expect(store.collectionStatus.subtasks).toBe('pending');
    expect(store.collectionStatus.checklist).toBe('pending');
    expect(store.collectionStatus.activity).toBe('pending');
    expect(store.collectionStatus.attachments).toBe('pending');
    expect(store.collectionStatus.comments).toBe('pending');

    for (const [index, request] of requests.slice(1).entries()) {
      request.resolve(collectionResponse(index + 1, 'Current'));
    }
    await load;
  });

  it('rejects stale comment pagination, activity reloads, and mutation rollbacks after a target change', async () => {
    const requests: Array<ReturnType<typeof deferred<unknown>>> = [];
    GET.mockImplementation(() => {
      const request = deferred<unknown>();
      requests.push(request);
      return request.promise;
    });

    const store = useTaskDetailStore();
    const firstLoad = store.loadAll('ws', 'ATL-1');
    for (const [index, request] of requests.slice(0, 8).entries()) {
      request.resolve(collectionResponse(index, 'First'));
    }
    await firstLoad;

    store._setForTest({
      comments: [comment('cm-first', 'First', 'user', 'Ann')],
      commentsCursor: 'cm-first',
      commentsHasMore: true,
      checklist: [checklistItem('c-first', 'First', false)],
    });

    const removeResponse = deferred<{ data?: undefined; error?: { hint: string } }>();
    DELETE.mockReturnValueOnce(removeResponse.promise);
    const page = store.loadMoreComments('ws', 'ATL-1');
    const remove = store.removeComment('ws', 'ATL-1', 'cm-first');
    POST.mockResolvedValueOnce({ data: checklistItem('c-created', 'Created', false), error: undefined });
    const addChecklist = await store.addChecklistItem('ws', 'ATL-1', 'Created');

    expect(addChecklist).toBe(true);

    const secondLoad = store.loadAll('ws', 'ATL-2');
    for (const [index, request] of requests.slice(10, 18).entries()) {
      request.resolve(collectionResponse(index, 'Second'));
    }
    await secondLoad;

    requests[8]?.resolve({
      data: { items: [comment('cm-stale', 'Stale page', 'user', 'Ann')], has_more: false, next_cursor: null },
      error: undefined,
    });
    requests[9]?.resolve({
      data: { items: [activityEntry('stale', 'stale', 'user', 'Ann')] },
      error: undefined,
    });
    removeResponse.resolve({ data: undefined, error: { hint: 'Stale remove failure' } });

    await page;

    expect(await remove).toBe(false);

    expect(store.comments.map((item) => item.body)).toEqual(['Second']);
    expect(store.activity[0]?.actor.display_name).toBe('Second');
    expect(store.error).toBeNull();
  });

  it('settles rejected current-target collection requests as area-local errors', async () => {
    const store = useTaskDetailStore();
    GET.mockRejectedValue(new Error('Network unavailable'));

    await store.loadAll('ws', 'ATL-1');

    expect(Object.values(store.collectionStatus)).toEqual(Array(8).fill('error'));
    expect(Object.values(store.collectionErrors)).toEqual(Array(8).fill('Failed to load task detail'));
    expect(store.loading).toBe(false);
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

  it('addChecklistItem re-fetches the activity feed so the new entry surfaces', async () => {
    const store = useTaskDetailStore();

    POST.mockResolvedValueOnce({
      data: checklistItem('c9', 'Write tests', false),
      error: undefined,
    });
    GET.mockResolvedValueOnce({
      data: { items: [{ id: 'a1', kind: 'checklist_added' }] },
      error: undefined,
    });

    const ok = await store.addChecklistItem('ws', 'ATL-1', 'Write tests');
    expect(ok).toBe(true);

    // The activity reload is fire-and-forget; let its microtask settle.
    await Promise.resolve();
    await Promise.resolve();

    expect(GET).toHaveBeenCalledWith('/api/workspaces/{ws}/tasks/{readable_id}/activity', {
      params: { path: { ws: 'ws', readable_id: 'ATL-1' } },
    });
    expect(store.activity.map((a) => a.kind)).toEqual(['checklist_added']);
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

  it.each([
    'moveSubtaskToColumn',
    'promoteSubtask',
  ] as const)('invalidates parent and child task cache scopes after %s succeeds', async (method) => {
    const store = useTaskDetailStore();
    GET.mockImplementation((path: string) =>
      Promise.resolve(collectionResponse(collectionIndex(path), 'Parent')),
    );
    await store.loadAll(
      'ws',
      'ATL-1',
      '018f0000-0000-7000-8000-000000000002',
      '018f0000-0000-7000-8000-000000000003',
    );
    store._setForTest({ subtasks: [subtaskSummary('s1', 'ATL-2', 'Child')] });
    POST.mockResolvedValueOnce({ data: undefined, error: undefined });

    const ok =
      method === 'moveSubtaskToColumn'
        ? await store.moveSubtaskToColumn('ws', 'ATL-2', 'col-2')
        : await store.promoteSubtask('ws', 'ATL-2');

    expect(ok).toBe(true);
    expect(cachePurgeTags).toHaveBeenCalledWith(
      '018f0000-0000-7000-8000-000000000002',
      'ATL-2',
      undefined,
      's1',
    );
    expect(cachePurgeTags).toHaveBeenCalledWith(
      '018f0000-0000-7000-8000-000000000002',
      'ATL-1',
      undefined,
      '018f0000-0000-7000-8000-000000000003',
    );
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

  it('removeReference removes a manual-only row after a successful deletion', async () => {
    const store = useTaskDetailStore();
    store._setForTest({ references: [reference('manual-1', 'relates')] });
    DELETE.mockResolvedValueOnce({ data: undefined, error: undefined });
    GET.mockResolvedValueOnce({ data: [], error: undefined });

    const ok = await store.removeReference('ws', 'ATL-1', 'manual-1');

    expect(ok).toBe(true);
    expect(store.references).toEqual([]);
    const [, options] = DELETE.mock.calls[0] as [string, { params: { path: { reference_id: string } } }];
    expect(options.params.path.reference_id).toBe('manual-1');
  });

  it('removeReference converts a merged row to a non-actionable wikilink before reconciliation', async () => {
    const store = useTaskDetailStore();
    store._setForTest({ references: [mergedReference('manual-1', 'link-1')] });
    DELETE.mockResolvedValueOnce({ data: undefined, error: undefined });
    GET.mockResolvedValueOnce({
      data: [wikilinkReference('link-1', 'Authoritative title')],
      error: undefined,
    });

    const removing = store.removeReference('ws', 'ATL-1', 'manual-1');

    expect(store.references).toEqual([wikilinkReference('link-1', 'Linked document')]);
    expect(await removing).toBe(true);
    expect(store.references).toEqual([wikilinkReference('link-1', 'Authoritative title')]);
    expect(GET).toHaveBeenCalledWith('/api/workspaces/{ws}/tasks/{readable_id}/references', {
      params: { path: { ws: 'ws', readable_id: 'ATL-1' } },
    });
  });

  it('removeReference keeps its optimistic state when reconciliation fails', async () => {
    const store = useTaskDetailStore();
    store._setForTest({ references: [mergedReference('manual-1', 'link-1')] });
    DELETE.mockResolvedValueOnce({ data: undefined, error: undefined });
    GET.mockResolvedValueOnce({ data: undefined, error: { hint: 'References unavailable' } });

    const ok = await store.removeReference('ws', 'ATL-1', 'manual-1');

    expect(ok).toBe(true);
    expect(store.references).toEqual([wikilinkReference('link-1', 'Linked document')]);
    expect(store.error).toBeNull();
  });

  it('removeReference restores the exact merged snapshot when deletion fails', async () => {
    const store = useTaskDetailStore();
    const merged = mergedReference('manual-1', 'link-1');
    store._setForTest({ references: [merged] });
    DELETE.mockResolvedValueOnce({ data: undefined, error: { hint: 'Cannot remove reference' } });

    const ok = await store.removeReference('ws', 'ATL-1', 'manual-1');

    expect(ok).toBe(false);
    expect(store.references).toEqual([merged]);
    expect(store.error).toBe('Cannot remove reference');
  });

  it('does not restore an old reference when a stale deletion fails', async () => {
    const store = useTaskDetailStore();
    const deleteResponse = deferred<{ data?: undefined; error?: { hint: string } }>();
    store._setForTest({ references: [reference('manual-old', 'relates')] });
    DELETE.mockReturnValueOnce(deleteResponse.promise);

    const removing = store.removeReference('ws', 'ATL-1', 'manual-old');
    store._setForTest({ references: [reference('manual-new', 'blocks')] });
    GET.mockImplementation((path: string) =>
      Promise.resolve({
        data: path.endsWith('/references') ? [reference('manual-new', 'blocks')] : undefined,
        error: undefined,
      }),
    );
    const replacementLoad = store.loadAll('ws', 'ATL-2');
    await Promise.resolve();

    deleteResponse.resolve({ data: undefined, error: { hint: 'Old deletion failed' } });
    await replacementLoad;

    expect(await removing).toBe(false);
    expect(store.references).toEqual([reference('manual-new', 'blocks')]);
    expect(store.error).toBeNull();
  });

  it('does not apply a stale reconciliation response over a new target', async () => {
    const store = useTaskDetailStore();
    const reloadResponse = deferred<{ data?: ReturnType<typeof wikilinkReference>[]; error?: undefined }>();
    store._setForTest({ references: [mergedReference('manual-old', 'link-old')] });
    DELETE.mockResolvedValueOnce({ data: undefined, error: undefined });
    GET.mockReturnValueOnce(reloadResponse.promise);

    const removing = store.removeReference('ws', 'ATL-1', 'manual-old');
    await Promise.resolve();
    store._setForTest({ references: [reference('manual-new', 'blocks')] });
    GET.mockImplementation((path: string) =>
      Promise.resolve({
        data: path.endsWith('/references') ? [reference('manual-new', 'blocks')] : undefined,
        error: undefined,
      }),
    );
    const replacementLoad = store.loadAll('ws', 'ATL-2');
    await Promise.resolve();

    reloadResponse.resolve({ data: [wikilinkReference('link-old', 'Stale title')], error: undefined });
    await replacementLoad;

    expect(await removing).toBe(false);
    expect(store.references).toEqual([reference('manual-new', 'blocks')]);
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
