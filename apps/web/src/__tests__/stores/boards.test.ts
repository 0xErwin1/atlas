import { flushPromises } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { type ZodType, z } from 'zod';
import { deferred } from '@/__tests__/deferred';
import {
  allowResourceCache,
  configureResourceCacheForTest,
  setResourceCachePrincipal,
} from '@/cache/cacheRuntime';
import {
  buildCacheKey,
  type CacheEnvelope,
  ResourceCache,
  type ResourceCacheStore,
} from '@/cache/resourceCache';

const { GET } = vi.hoisted(() => ({ GET: vi.fn() }));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET },
}));

import type { BoardDto, BoardSummaryDto, ColumnDto, TaskSummaryDto } from '@/stores/boards';
import { useBoardsStore } from '@/stores/boards';

const PRINCIPAL = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08';
const WORKSPACE_ID = '019ef171-bbcf-7b90-9be6-5dbb382afd08';

function cacheStore(entries: Map<string, CacheEnvelope<unknown>>): ResourceCacheStore {
  return {
    async get<T>(key: string, _schema: ZodType<T>): Promise<CacheEnvelope<T> | null> {
      const entry = entries.get(key);
      return entry === undefined ? null : (entry as CacheEnvelope<T>);
    },
    async putMany(newEntries) {
      for (const entry of newEntries) entries.set(entry.key, entry);
      return true;
    },
    async deleteMany(keys) {
      for (const key of keys) entries.delete(key);
      return true;
    },
    async deleteScope(scope) {
      for (const [key, entry] of entries) {
        const inWorkspace =
          key.includes(`|p=${scope.principal}|`) &&
          (scope.workspaceId === undefined || key.includes(`|w=${scope.workspaceId}|`));
        const hasTag = scope.tagsAny === undefined || entry.tags.some((tag) => scope.tagsAny?.includes(tag));
        if (inWorkspace && hasTag) entries.delete(key);
      }
      return true;
    },
    async clear() {
      return true;
    },
  };
}

const board = (id: string, workspaceId = WORKSPACE_ID): BoardDto => ({
  id,
  name: `Board ${id}`,
  workspace_id: workspaceId,
  project_id: 'proj',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
  created_by: { id: 'u1', type: 'user', display_name: 'User' },
});

const col = (id: string, positionKey: string): ColumnDto => ({
  id,
  board_id: 'board-1',
  name: `Col ${id}`,
  position_key: positionKey,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
});

const boardSummary = (id: string, taskCount: number, folderId: string | null = null): BoardSummaryDto => ({
  id,
  name: `Board ${id}`,
  folder_id: folderId,
  task_count: taskCount,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
});

const task = (id: string, readableId: string, columnId: string): TaskSummaryDto => ({
  id,
  readable_id: readableId,
  board_id: 'board-1',
  column_id: columnId,
  board_name: 'Board',
  column_name: 'Todo',
  title: `Task ${id}`,
  priority: null,
  subtask_count: 0,
  updated_at: '2026-01-01T00:00:00Z',
});

describe('useBoardsStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    setResourceCachePrincipal(PRINCIPAL);
    configureResourceCacheForTest(new ResourceCache({ store: cacheStore(new Map()) }));
    allowResourceCache();
  });

  it('returns a stable array reference for an empty column (guards the kanban render loop)', () => {
    const store = useBoardsStore();
    const first = store.tasksByColumn('empty-col');
    const second = store.tasksByColumn('empty-col');

    expect(first).toEqual([]);
    expect(first).toBe(second);
  });

  it('loadBoard fetches the board and stores it (REQ-W20)', async () => {
    GET.mockResolvedValueOnce({
      data: {
        id: 'board-1',
        name: 'Sprint 1',
        workspace_id: 'ws',
        project_id: 'proj',
        created_at: '',
        updated_at: '',
        created_by: { id: 'u1', name: 'User', principal_type: 'user' },
      },
      error: undefined,
    });

    const store = useBoardsStore();
    await store.loadBoard('ws', 'board-1');

    expect(store.board?.id).toBe('board-1');
    expect(store.board?.name).toBe('Sprint 1');
    expect(store.error).toBeNull();
  });

  it('loadColumns stores columns sorted by position_key (REQ-W20)', async () => {
    GET.mockResolvedValue({
      data: [col('c2', 'n'), col('c1', 'a'), col('c3', 'z')],
      error: undefined,
    });

    const store = useBoardsStore();
    await store.loadColumns('ws', 'board-1');

    expect(store.columns).toHaveLength(3);
    expect(store.columns[0]?.id).toBe('c1');
    expect(store.columns[1]?.id).toBe('c2');
    expect(store.columns[2]?.id).toBe('c3');
  });

  it('loadTasks groups tasks by column and preserves server order (REQ-W20)', async () => {
    GET.mockResolvedValue({
      data: {
        items: [task('t1', 'ATL-1', 'c1'), task('t2', 'ATL-2', 'c1'), task('t3', 'ATL-3', 'c2')],
        has_more: false,
        next_cursor: null,
      },
      error: undefined,
    });

    const store = useBoardsStore();
    await store.loadTasks('ws', 'board-1');

    const c1Tasks = store.tasksByColumn('c1');
    const c2Tasks = store.tasksByColumn('c2');

    expect(c1Tasks).toHaveLength(2);
    expect(c1Tasks[0]?.id).toBe('t1');
    expect(c1Tasks[1]?.id).toBe('t2');
    expect(c2Tasks).toHaveLength(1);
    expect(c2Tasks[0]?.id).toBe('t3');
  });

  it('loadTasks surfaces hint on error', async () => {
    GET.mockResolvedValue({ data: undefined, error: { hint: 'board not found' } });

    const store = useBoardsStore();
    await store.loadTasks('ws', 'board-1');

    expect(store.loadError).toBe('board not found');
  });

  it('keeps only the newest coordinated board load when responses resolve out of order', async () => {
    const responses = new Map<string, ReturnType<typeof deferred<{ data: unknown; error: undefined }>>>();
    for (const boardId of ['board-1', 'board-2']) {
      for (const resource of ['board', 'columns', 'tasks']) {
        responses.set(`${boardId}:${resource}`, deferred());
      }
    }
    GET.mockImplementation((path: string, request: { params: { path: { board_id: string } } }) => {
      const boardId = request.params.path.board_id;
      const resource = path.endsWith('/columns') ? 'columns' : path.endsWith('/tasks') ? 'tasks' : 'board';
      return responses.get(`${boardId}:${resource}`)?.promise;
    });

    const store = useBoardsStore();
    store.columns = [col('old-column', 'a')];
    store._setTasksForTest({ 'old-column': [task('old-task', 'ATL-1', 'old-column')] });

    const firstLoad = store.loadBoardContents('ws', 'board-1');
    const secondLoad = store.loadBoardContents('ws', 'board-2');

    expect(store.loading).toBe(true);
    expect(store.columns).toEqual([]);
    expect(store.tasksByColumn('old-column')).toEqual([]);

    responses.get('board-2:board')?.resolve({ data: board('board-2'), error: undefined });
    responses.get('board-2:columns')?.resolve({ data: [col('new-column', 'b')], error: undefined });
    responses.get('board-2:tasks')?.resolve({
      data: {
        items: [task('new-task', 'ATL-2', 'new-column')],
        has_more: false,
        next_cursor: null,
      },
      error: undefined,
    });
    await secondLoad;

    expect(store.loading).toBe(false);
    expect(store.board?.id).toBe('board-2');
    expect(store.columns.map((column) => column.id)).toEqual(['new-column']);
    expect(store.tasksByColumn('new-column').map((item) => item.id)).toEqual(['new-task']);

    responses.get('board-1:board')?.resolve({ data: board('board-1'), error: undefined });
    responses.get('board-1:columns')?.resolve({ data: [col('stale-column', 'a')], error: undefined });
    responses.get('board-1:tasks')?.resolve({
      data: {
        items: [task('stale-task', 'ATL-3', 'stale-column')],
        has_more: false,
        next_cursor: null,
      },
      error: undefined,
    });
    await firstLoad;

    expect(store.board?.id).toBe('board-2');
    expect(store.columns.map((column) => column.id)).toEqual(['new-column']);
    expect(store.tasksByColumn('new-column').map((item) => item.id)).toEqual(['new-task']);
  });

  it('background refresh keeps the mounted board and swaps atomically without a loading flip', async () => {
    const boardResponse = deferred<{ data: BoardDto; error: undefined }>();
    const columnsResponse = deferred<{ data: ColumnDto[]; error: undefined }>();
    const tasksResponse = deferred<{
      data: { items: TaskSummaryDto[]; has_more: false; next_cursor: null };
      error: undefined;
    }>();
    GET.mockImplementation((path: string) => {
      if (path.endsWith('/columns')) return columnsResponse.promise;
      if (path.endsWith('/tasks')) return tasksResponse.promise;
      return boardResponse.promise;
    });

    const store = useBoardsStore();
    store.board = board('board-1');
    store.columns = [col('old-column', 'a')];
    store._setTasksForTest({ 'old-column': [task('old-task', 'ATL-1', 'old-column')] });

    const refresh = store.loadBoardContents('ws', 'board-1', undefined, { background: true });

    expect(store.loading).toBe(false);
    expect(store.board?.id).toBe('board-1');
    expect(store.tasksByColumn('old-column').map((item) => item.id)).toEqual(['old-task']);

    boardResponse.resolve({ data: board('board-1'), error: undefined });
    columnsResponse.resolve({ data: [col('new-column', 'b')], error: undefined });
    tasksResponse.resolve({
      data: { items: [task('new-task', 'ATL-2', 'new-column')], has_more: false, next_cursor: null },
      error: undefined,
    });
    await refresh;

    expect(store.loading).toBe(false);
    expect(store.columns.map((column) => column.id)).toEqual(['new-column']);
    expect(store.tasksByColumn('new-column').map((item) => item.id)).toEqual(['new-task']);
  });

  it('background refresh preserves the mounted board on a transient failure', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    GET.mockResolvedValue({ data: undefined, error: { status: 500, hint: 'server error' } });

    const store = useBoardsStore();
    store.board = board('board-1');
    store.columns = [col('old-column', 'a')];
    store._setTasksForTest({ 'old-column': [task('old-task', 'ATL-1', 'old-column')] });

    await store.loadBoardContents('ws', 'board-1', undefined, { background: true });

    expect(store.board?.id).toBe('board-1');
    expect(store.tasksByColumn('old-column').map((item) => item.id)).toEqual(['old-task']);
    expect(store.loading).toBe(false);
    expect(store.loadError).toBeNull();
    expect(warn).toHaveBeenCalledOnce();

    warn.mockRestore();
  });

  it('hydrates an exact board composite before an offline refresh and keeps it usable for active retry', async () => {
    const key = buildCacheKey({
      principal: PRINCIPAL,
      workspaceId: WORKSPACE_ID,
      resourceKind: 'task-board',
      resourceId: 'board-2',
    });
    if (key === null) throw new Error('Expected a canonical board cache key');

    const now = Date.now();
    const entries = new Map<string, CacheEnvelope<unknown>>([
      [
        key,
        {
          schema: 1,
          key,
          payloadVersion: 1,
          storedAt: now,
          validatedAt: now,
          lastAccessedAt: now,
          retentionExpiresAt: now + 60_000,
          bytes: 128,
          stale: false,
          tags: ['board:board-2'],
          payload: {
            board: board('board-2'),
            columns: [{ ...col('cached-column', 'a'), board_id: 'board-2' }],
            tasks: [{ ...task('cached-task', 'ATL-2', 'cached-column'), board_id: 'board-2' }],
          },
        },
      ],
    ]);
    const cache = new ResourceCache({ store: cacheStore(entries) });
    configureResourceCacheForTest(cache);
    allowResourceCache();
    GET.mockResolvedValue({ data: undefined, error: { hint: 'offline' } });

    const store = useBoardsStore();
    await store.loadBoardContents('ws', 'board-2', WORKSPACE_ID);

    expect(store.loading).toBe(false);
    expect(store.board?.id).toBe('board-2');
    expect(store.columns.map((column) => column.id)).toEqual(['cached-column']);
    expect(store.tasksByColumn('cached-column').map((item) => item.id)).toEqual(['cached-task']);
    expect(store.loadError).toBe('offline');

    GET.mockResolvedValue({
      data: { items: [task('fresh-task', 'ATL-3', 'fresh-column')], has_more: false, next_cursor: null },
      error: undefined,
    });
    await cache.retry(key);

    expect(store.tasksByColumn('cached-column').map((item) => item.id)).toEqual(['cached-task']);
  });

  it.each([
    403, 404,
  ])('evicts the exact board composite after a %i response before it can hydrate again', async (status) => {
    const key = buildCacheKey({
      principal: PRINCIPAL,
      workspaceId: WORKSPACE_ID,
      resourceKind: 'task-board',
      resourceId: 'board-1',
    });
    if (key === null) throw new Error('Expected a canonical board cache key');

    const now = Date.now();
    const entries = new Map<string, CacheEnvelope<unknown>>([
      [
        key,
        {
          schema: 1,
          key,
          payloadVersion: 1,
          storedAt: now,
          validatedAt: now,
          lastAccessedAt: now,
          retentionExpiresAt: now + 60_000,
          bytes: 128,
          stale: false,
          tags: ['board:board-1'],
          payload: {
            board: board('board-1'),
            columns: [col('cached-column', 'a')],
            tasks: [task('cached-task', 'ATL-1', 'cached-column')],
          },
        },
      ],
    ]);
    const cache = new ResourceCache({ store: cacheStore(entries) });
    configureResourceCacheForTest(cache);
    allowResourceCache();
    GET.mockResolvedValue({ data: undefined, error: { status, hint: 'Denied' } });

    const store = useBoardsStore();
    await store.loadBoardContents('ws', 'board-1', WORKSPACE_ID);

    expect(store.board).toBeNull();
    expect(entries.has(key)).toBe(false);

    GET.mockImplementation((path: string) => {
      if (path.endsWith('/columns'))
        return Promise.resolve({ data: [col('fresh-column', 'a')], error: undefined });
      if (path.endsWith('/tasks')) {
        return Promise.resolve({
          data: { items: [task('fresh-task', 'ATL-2', 'fresh-column')], has_more: false, next_cursor: null },
          error: undefined,
        });
      }
      return Promise.resolve({ data: board('board-1'), error: undefined });
    });
    await store.loadBoardContents('ws', 'board-1', WORKSPACE_ID);

    expect(store.tasksByColumn('fresh-column').map((item) => item.id)).toEqual(['fresh-task']);
    expect(store.tasksByColumn('cached-column')).toEqual([]);
  });

  it.each([
    'columns',
    'tasks',
  ] as const)('retracts and evicts the board composite when %s is denied', async (deniedResource) => {
    const entries = new Map<string, CacheEnvelope<unknown>>();
    configureResourceCacheForTest(new ResourceCache({ store: cacheStore(entries) }));
    allowResourceCache();
    GET.mockImplementation((path: string) => {
      if (path.endsWith(`/${deniedResource}`)) {
        return Promise.resolve({ data: undefined, error: { status: 403, hint: 'Denied' } });
      }
      if (path.endsWith('/columns'))
        return Promise.resolve({ data: [col('column-1', 'a')], error: undefined });
      if (path.endsWith('/tasks')) {
        return Promise.resolve({
          data: { items: [task('task-1', 'ATL-1', 'column-1')], has_more: false, next_cursor: null },
          error: undefined,
        });
      }
      return Promise.resolve({ data: board('board-1'), error: undefined });
    });

    const store = useBoardsStore();
    await store.loadBoardContents('ws', 'board-1', WORKSPACE_ID);

    expect(store.board).toBeNull();
    expect(store.columns).toEqual([]);
    expect(store.tasksByColumn('column-1')).toEqual([]);
    expect(entries.size).toBe(0);
  });

  it('rejects a swapped board composite instead of publishing it under the requested board key', async () => {
    GET.mockImplementation((path: string) => {
      if (path.endsWith('/columns'))
        return Promise.resolve({ data: [col('column-1', 'a')], error: undefined });
      if (path.endsWith('/tasks')) {
        return Promise.resolve({
          data: { items: [task('task-1', 'ATL-1', 'column-1')], has_more: false, next_cursor: null },
          error: undefined,
        });
      }
      return Promise.resolve({ data: board('another-board'), error: undefined });
    });

    const store = useBoardsStore();
    await store.loadBoardContents('ws', 'board-1', WORKSPACE_ID);

    expect(store.board).toBeNull();
    expect(store.columns).toEqual([]);
    expect(store.tasksByColumn('column-1')).toEqual([]);
  });

  it('rejects a swapped board composite during the direct online fallback', async () => {
    configureResourceCacheForTest({ isAvailable: () => false });
    GET.mockImplementation((path: string) => {
      if (path.endsWith('/columns'))
        return Promise.resolve({ data: [col('column-1', 'a')], error: undefined });
      if (path.endsWith('/tasks')) {
        return Promise.resolve({
          data: { items: [task('task-1', 'ATL-1', 'column-1')], has_more: false, next_cursor: null },
          error: undefined,
        });
      }
      return Promise.resolve({ data: board('another-board'), error: undefined });
    });

    const store = useBoardsStore();
    await store.loadBoardContents('ws', 'board-1', WORKSPACE_ID);

    expect(store.board).toBeNull();
    expect(store.columns).toEqual([]);
    expect(store.tasksByColumn('column-1')).toEqual([]);
  });

  it('evicts the active exact board composite after a task mutation', async () => {
    const entries = new Map<string, CacheEnvelope<unknown>>();
    const cache = new ResourceCache({ store: cacheStore(entries) });
    configureResourceCacheForTest(cache);
    allowResourceCache();
    GET.mockImplementation((path: string) => {
      if (path.endsWith('/columns'))
        return Promise.resolve({ data: [col('column-1', 'a')], error: undefined });
      if (path.endsWith('/tasks')) {
        return Promise.resolve({
          data: { items: [task('task-1', 'ATL-1', 'column-1')], has_more: false, next_cursor: null },
          error: undefined,
        });
      }
      return Promise.resolve({ data: board('board-1'), error: undefined });
    });

    const key = buildCacheKey({
      principal: PRINCIPAL,
      workspaceId: WORKSPACE_ID,
      resourceKind: 'task-board',
      resourceId: 'board-1',
    });
    if (key === null) throw new Error('Expected a canonical board cache key');

    const store = useBoardsStore();
    await store.loadBoardContents('ws', 'board-1', WORKSPACE_ID);
    store.reconcileTask({
      id: 'task-1',
      readable_id: 'ATL-1',
      column_id: 'column-1',
      title: 'Updated task',
      priority: null,
      updated_at: '2026-01-02T00:00:00Z',
    });
    await vi.waitFor(() => expect(entries.has(key)).toBe(false));

    expect(
      await cache.hydrate({ key, payloadSchema: z.unknown(), publish: vi.fn(), isCurrent: () => true }),
    ).toBeNull();
  });

  it('isolates a coordinated board load from standalone loaders', async () => {
    const boardResponse = deferred<{ data: unknown; error: undefined }>();
    const columnsResponse = deferred<{ data: unknown; error: undefined }>();
    const tasksResponse = deferred<{ data: unknown; error: undefined }>();
    GET.mockImplementation((path: string, request: { params: { path: { board_id: string } } }) => {
      if (request.params.path.board_id === 'board-1') {
        if (path.endsWith('/columns')) {
          return Promise.resolve({ data: [col('stale-column', 'a')], error: undefined });
        }
        if (path.endsWith('/tasks')) {
          return Promise.resolve({
            data: {
              items: [task('stale-task', 'ATL-1', 'stale-column')],
              has_more: false,
              next_cursor: null,
            },
            error: undefined,
          });
        }
        return Promise.resolve({ data: board('board-1'), error: undefined });
      }
      if (path.endsWith('/columns')) return columnsResponse.promise;
      if (path.endsWith('/tasks')) return tasksResponse.promise;
      return boardResponse.promise;
    });

    const store = useBoardsStore();
    const coordinatedLoad = store.loadBoardContents('ws', 'board-2');
    await Promise.all([
      store.loadBoard('ws', 'board-1'),
      store.loadColumns('ws', 'board-1'),
      store.loadTasks('ws', 'board-1'),
    ]);

    expect(store.loading).toBe(true);
    expect(store.board).toBeNull();

    boardResponse.resolve({ data: board('board-2'), error: undefined });
    columnsResponse.resolve({ data: [col('new-column', 'a')], error: undefined });
    tasksResponse.resolve({
      data: {
        items: [task('new-task', 'ATL-2', 'new-column')],
        has_more: false,
        next_cursor: null,
      },
      error: undefined,
    });
    await coordinatedLoad;

    expect(store.loading).toBe(false);
    expect(store.board?.id).toBe('board-2');
    expect(store.columns.map((column) => column.id)).toEqual(['new-column']);
    expect(store.tasksByColumn('new-column').map((item) => item.id)).toEqual(['new-task']);
  });

  it('replays a live column refresh requested during a coordinated load', async () => {
    const boardResponse = deferred<{ data: unknown; error: undefined }>();
    const initialColumnsResponse = deferred<{ data: unknown; error: undefined }>();
    const refreshedColumnsResponse = deferred<{ data: unknown; error: undefined }>();
    const tasksResponse = deferred<{ data: unknown; error: undefined }>();
    let columnRequestCount = 0;
    GET.mockImplementation((path: string) => {
      if (path.endsWith('/columns')) {
        columnRequestCount += 1;
        return columnRequestCount === 1 ? initialColumnsResponse.promise : refreshedColumnsResponse.promise;
      }
      if (path.endsWith('/tasks')) return tasksResponse.promise;
      return boardResponse.promise;
    });

    const store = useBoardsStore();
    const load = store.loadBoardContents('ws', 'board-1');
    await store.loadColumns('ws', 'board-1');

    boardResponse.resolve({ data: board('board-1'), error: undefined });
    initialColumnsResponse.resolve({ data: [col('stale-column', 'a')], error: undefined });
    tasksResponse.resolve({ data: { items: [], has_more: false, next_cursor: null }, error: undefined });
    await flushPromises();

    expect(store.loading).toBe(true);
    expect(columnRequestCount).toBe(2);

    refreshedColumnsResponse.resolve({ data: [col('fresh-column', 'b')], error: undefined });
    await load;

    expect(store.loading).toBe(false);
    expect(store.columns.map((column) => column.id)).toEqual(['fresh-column']);
  });

  it('publishes a coordinated load error only after all resources settle', async () => {
    const boardResponse = deferred<{ data: unknown; error: { hint: string } }>();
    const columnsResponse = deferred<{ data: unknown; error: undefined }>();
    const tasksResponse = deferred<{ data: unknown; error: undefined }>();
    GET.mockImplementation((path: string) => {
      if (path.endsWith('/columns')) return columnsResponse.promise;
      if (path.endsWith('/tasks')) return tasksResponse.promise;
      return boardResponse.promise;
    });

    const store = useBoardsStore();
    const load = store.loadBoardContents('ws', 'board-1');

    boardResponse.resolve({ data: undefined, error: { hint: 'board unavailable' } });
    tasksResponse.reject(new Error('network disconnected'));
    await Promise.resolve();

    expect(store.loading).toBe(true);
    expect(store.loadError).toBeNull();

    columnsResponse.resolve({ data: [col('column-1', 'a')], error: undefined });
    await load;

    expect(store.loading).toBe(false);
    expect(store.loadError).toBe('board unavailable');
    expect(store.board).toBeNull();
    expect(store.columns).toEqual([]);
  });

  it('invalidates an in-flight task detail fan-out when a new board load starts', async () => {
    const detailResponse = deferred<{ data: unknown; error: undefined }>();
    GET.mockImplementation((path: string) => {
      if (path.includes('/tasks/ATL-1')) return detailResponse.promise;
      if (path.endsWith('/columns')) return Promise.resolve({ data: [], error: undefined });
      if (path.endsWith('/tasks')) {
        return Promise.resolve({
          data: { items: [], has_more: false, next_cursor: null },
          error: undefined,
        });
      }
      return Promise.resolve({ data: board('board-2'), error: undefined });
    });

    const store = useBoardsStore();
    store._setTasksForTest({ 'old-column': [task('old-task', 'ATL-1', 'old-column')] });
    const detailLoad = store.loadTaskDetails('ws');

    await store.loadBoardContents('ws', 'board-2');
    detailResponse.resolve({
      data: {
        id: 'old-task',
        readable_id: 'ATL-1',
        title: 'Old task',
      },
      error: undefined,
    });
    await detailLoad;

    expect(store.taskDetails.size).toBe(0);
    expect(store.detailsLoading).toBe(false);
  });

  it('surfaces resolved task detail API errors without keeping stale details', async () => {
    GET.mockResolvedValueOnce({
      data: { id: 'task-1', readable_id: 'ATL-1', title: 'Task' },
      error: undefined,
    });

    const store = useBoardsStore();
    store._setTasksForTest({ 'column-1': [task('task-1', 'ATL-1', 'column-1')] });
    await store.loadTaskDetails('ws');
    expect(store.taskDetails.size).toBe(1);

    GET.mockResolvedValueOnce({ data: undefined, error: { hint: 'detail unavailable' } });
    await store.loadTaskDetails('ws');

    expect(store.detailsLoading).toBe(false);
    expect(store.taskDetails.size).toBe(0);
    expect(store.error).toBe('detail unavailable');
  });

  it('reset invalidates a pending coordinated board load', async () => {
    const boardResponse = deferred<{ data: unknown; error: undefined }>();
    const columnsResponse = deferred<{ data: unknown; error: undefined }>();
    const tasksResponse = deferred<{ data: unknown; error: undefined }>();
    GET.mockImplementation((path: string) => {
      if (path.endsWith('/columns')) return columnsResponse.promise;
      if (path.endsWith('/tasks')) return tasksResponse.promise;
      return boardResponse.promise;
    });

    const store = useBoardsStore();
    const load = store.loadBoardContents('ws', 'board-1');
    store.reset();

    boardResponse.resolve({ data: board('board-1'), error: undefined });
    columnsResponse.resolve({ data: [col('column-1', 'a')], error: undefined });
    tasksResponse.resolve({
      data: { items: [task('task-1', 'ATL-1', 'column-1')], has_more: false, next_cursor: null },
      error: undefined,
    });
    await load;

    expect(store.loading).toBe(false);
    expect(store.board).toBeNull();
    expect(store.columns).toEqual([]);
    expect(store.tasksByColumn('column-1')).toEqual([]);
  });

  it('reset clears a stale load error and board-scoped state (cross-workspace bleed)', async () => {
    GET.mockResolvedValue({ data: undefined, error: { hint: 'board not found' } });

    const store = useBoardsStore();
    await store.loadTasks('ws', 'board-1');
    store._setTasksForTest({ c1: [task('t1', 'ATL-1', 'c1')] });

    expect(store.loadError).toBe('board not found');

    store.reset();

    expect(store.loadError).toBeNull();
    expect(store.board).toBeNull();
    expect(store.columns).toHaveLength(0);
    expect(store.tasksByColumn('c1')).toHaveLength(0);
  });

  it('reconcileTask moves a task to a new column and updates its id (REQ-W21)', () => {
    const store = useBoardsStore();

    store._setTasksForTest({
      c1: [task('t1', 'ATL-1', 'c1'), task('t2', 'ATL-2', 'c1')],
      c2: [task('t3', 'ATL-3', 'c2')],
    });

    store.reconcileTask({
      id: 't1',
      readable_id: 'ATL-1',
      column_id: 'c2',
      title: 'Task t1',
      priority: 'high',
      updated_at: '2026-01-02T00:00:00Z',
    });

    expect(store.tasksByColumn('c1')).toHaveLength(1);
    expect(store.tasksByColumn('c1')[0]?.id).toBe('t2');
    expect(store.tasksByColumn('c2')).toHaveLength(2);
    const movedTask = store.tasksByColumn('c2').find((t) => t.id === 't1');
    expect(movedTask).toBeDefined();
    expect(movedTask?.priority).toBe('high');
  });

  it('reconcileTask handles a task moving to an empty column', () => {
    const store = useBoardsStore();

    store._setTasksForTest({
      c1: [task('t1', 'ATL-1', 'c1')],
      c2: [],
    });

    store.reconcileTask({
      id: 't1',
      readable_id: 'ATL-1',
      column_id: 'c2',
      title: 'Task t1',
      priority: null,
      updated_at: '2026-01-02T00:00:00Z',
    });

    expect(store.tasksByColumn('c1')).toHaveLength(0);
    expect(store.tasksByColumn('c2')).toHaveLength(1);
  });

  it('applyOptimisticMove moves task to target column at the given index', () => {
    const store = useBoardsStore();

    store._setTasksForTest({
      c1: [task('t1', 'ATL-1', 'c1'), task('t2', 'ATL-2', 'c1')],
      c2: [task('t3', 'ATL-3', 'c2')],
    });

    store.applyOptimisticMove('t1', 'c2', 0);

    const c1 = store.tasksByColumn('c1');
    const c2 = store.tasksByColumn('c2');

    expect(c1).toHaveLength(1);
    expect(c1[0]?.id).toBe('t2');
    expect(c2).toHaveLength(2);
    expect(c2[0]?.id).toBe('t1');
    expect(c2[1]?.id).toBe('t3');
  });

  it('snapshotTasks returns a deep copy that does not reflect subsequent mutations', () => {
    const store = useBoardsStore();

    store._setTasksForTest({
      c1: [task('t1', 'ATL-1', 'c1')],
    });

    const snapshot = store.snapshotTasks();
    store.applyOptimisticMove('t1', 'c2', 0);

    const snapshotC1 = snapshot.get('c1');
    expect(snapshotC1).toHaveLength(1);
    expect(snapshotC1?.[0]?.id).toBe('t1');
  });

  it('restoreSnapshot reverts to the captured state exactly', () => {
    const store = useBoardsStore();

    store._setTasksForTest({
      c1: [task('t1', 'ATL-1', 'c1'), task('t2', 'ATL-2', 'c1')],
      c2: [task('t3', 'ATL-3', 'c2')],
    });

    const snapshot = store.snapshotTasks();
    store.applyOptimisticMove('t1', 'c2', 0);

    expect(store.tasksByColumn('c1')).toHaveLength(1);

    store.restoreSnapshot(snapshot);

    expect(store.tasksByColumn('c1')).toHaveLength(2);
    expect(store.tasksByColumn('c1')[0]?.id).toBe('t1');
    expect(store.tasksByColumn('c1')[1]?.id).toBe('t2');
    expect(store.tasksByColumn('c2')).toHaveLength(1);
    expect(store.tasksByColumn('c2')[0]?.id).toBe('t3');
  });

  it('loadBoardsForProject exposes task_count on the summaries returned by boardsFor', async () => {
    GET.mockResolvedValue({
      data: { items: [boardSummary('b1', 3), boardSummary('b2', 0)], has_more: false, next_cursor: null },
      error: undefined,
    });

    const store = useBoardsStore();
    await store.loadBoardsForProject('ws', 'proj-a');

    const summaries = store.boardsFor('proj-a');
    expect(summaries).toHaveLength(2);
    expect(summaries.find((b) => b.id === 'b1')?.task_count).toBe(3);
    expect(summaries.find((b) => b.id === 'b2')?.task_count).toBe(0);
  });

  it('loadBoardsForProject keeps each project bucket independent when loading multiple projects', async () => {
    GET.mockImplementation((_path: string, request: { params: { path: { project_slug: string } } }) => {
      const slug = request.params.path.project_slug;
      const items = slug === 'proj-a' ? [boardSummary('a-1', 1)] : [boardSummary('b-1', 5)];
      return Promise.resolve({ data: { items, has_more: false, next_cursor: null }, error: undefined });
    });

    const store = useBoardsStore();
    await Promise.all([
      store.loadBoardsForProject('ws', 'proj-a'),
      store.loadBoardsForProject('ws', 'proj-b'),
    ]);

    expect(store.boardsFor('proj-a').map((b) => b.id)).toEqual(['a-1']);
    expect(store.boardsFor('proj-b').map((b) => b.id)).toEqual(['b-1']);
  });

  it('publishForProject sets boardsFor synchronously without a network call, preserving task_count', () => {
    const store = useBoardsStore();

    store.publishForProject('proj-a', [boardSummary('b1', 7)]);

    expect(GET).not.toHaveBeenCalled();
    expect(store.boardsFor('proj-a')).toHaveLength(1);
    expect(store.boardsFor('proj-a')[0]?.task_count).toBe(7);
  });
});
