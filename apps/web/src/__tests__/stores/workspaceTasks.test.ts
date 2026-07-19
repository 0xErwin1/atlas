import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ZodType } from 'zod';
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

const PRINCIPAL = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08';
const WORKSPACE_ID = '019ef171-bbcf-7b90-9be6-5dbb382afd08';

vi.mock('@/api/wrapper', () => ({ wrappedClient: { GET } }));

import type { TaskSummaryDto } from '@/stores/workspaceTasks';
import { useWorkspaceTasksStore } from '@/stores/workspaceTasks';

const task = (id: string): TaskSummaryDto => ({
  id,
  readable_id: `ATL-${id}`,
  board_id: 'board-1',
  board_name: 'Board',
  column_id: 'column-1',
  column_name: 'Todo',
  title: `Task ${id}`,
  priority: null,
  subtask_count: 0,
  updated_at: '2026-01-01T00:00:00Z',
});

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
        const inScope =
          key.includes(`|p=${scope.principal}|`) &&
          (scope.workspaceId === undefined || key.includes(`|w=${scope.workspaceId}|`));
        const matchesTag =
          scope.tagsAny === undefined || entry.tags.some((tag) => scope.tagsAny?.includes(tag));
        if (inScope && matchesTag) entries.delete(key);
      }
      return true;
    },
    async clear() {
      return true;
    },
  };
}

describe('useWorkspaceTasksStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    setResourceCachePrincipal(PRINCIPAL);
    configureResourceCacheForTest(new ResourceCache({ store: cacheStore(new Map()) }));
    allowResourceCache();
  });

  it('background refresh keeps the mounted list and swaps atomically without a loading flip', async () => {
    GET.mockResolvedValueOnce({
      data: { items: [task('a')], has_more: false, next_cursor: null },
      error: undefined,
    });

    const store = useWorkspaceTasksStore();
    await store.load('ws', { assignee: 'me' });

    expect(store.tasks.map((item) => item.id)).toEqual(['a']);
    expect(store.hasData).toBe(true);

    const pending = deferred<{
      data: { items: TaskSummaryDto[]; has_more: false; next_cursor: null };
      error: undefined;
    }>();
    GET.mockReturnValueOnce(pending.promise);

    const refresh = store.load('ws', { assignee: 'me' }, true, undefined, { background: true });

    expect(store.loading).toBe(false);
    expect(store.tasks.map((item) => item.id)).toEqual(['a']);
    expect(store.hasData).toBe(true);

    pending.resolve({
      data: { items: [task('b')], has_more: false, next_cursor: null },
      error: undefined,
    });
    await refresh;

    expect(store.loading).toBe(false);
    expect(store.tasks.map((item) => item.id)).toEqual(['b']);
  });

  it('background refresh preserves the mounted list on a transient failure', async () => {
    GET.mockResolvedValueOnce({
      data: { items: [task('a')], has_more: false, next_cursor: null },
      error: undefined,
    });

    const store = useWorkspaceTasksStore();
    await store.load('ws', { assignee: 'me' });

    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    GET.mockResolvedValueOnce({ data: undefined, error: { status: 500, hint: 'server error' } });

    await store.load('ws', { assignee: 'me' }, true, undefined, { background: true });

    expect(store.tasks.map((item) => item.id)).toEqual(['a']);
    expect(store.hasData).toBe(true);
    expect(store.loading).toBe(false);
    expect(store.error).toBeNull();
    expect(warn).toHaveBeenCalledOnce();

    warn.mockRestore();
  });

  it('hydrates only the exact normalized task query before an offline refresh', async () => {
    const key = buildCacheKey({
      principal: PRINCIPAL,
      workspaceId: WORKSPACE_ID,
      resourceKind: 'task-list',
      resourceId: 'workspace-tasks',
      query: { label: ['frontend', 'bug'], limit: 200, priority: ['high', 'low'], sort: 'title_asc' },
      setValuedQueryKeys: ['label', 'priority'],
    });
    if (key === null) throw new Error('Expected a canonical workspace task cache key');

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
          tags: ['workspace-tasks'],
          payload: { items: [task('cached')], has_more: true, next_cursor: 'cursor-2' },
        },
      ],
    ]);
    configureResourceCacheForTest(new ResourceCache({ store: cacheStore(entries) }));
    allowResourceCache();
    GET.mockResolvedValue({ data: undefined, error: { hint: 'offline' } });

    const store = useWorkspaceTasksStore();
    await store.load(
      'ws',
      { label: ['bug', 'frontend'], priority: ['low', 'high'], sort: 'title_asc' },
      false,
      WORKSPACE_ID,
    );

    expect(store.tasks.map((item) => item.id)).toEqual(['cached']);
    expect(store.hasMore).toBe(true);
    expect(store.nextCursor).toBe('cursor-2');
    expect(store.error).toBe('offline');
  });

  it.each([403, 404])('retracts and evicts an exact cached query after a known %i denial', async (status) => {
    const entries = new Map<string, CacheEnvelope<unknown>>();
    configureResourceCacheForTest(new ResourceCache({ store: cacheStore(entries) }));
    allowResourceCache();
    GET.mockResolvedValueOnce({
      data: { items: [task('cached')], has_more: false, next_cursor: null },
      error: undefined,
    });
    GET.mockResolvedValueOnce({ data: undefined, error: { status, hint: 'Denied' } });

    const store = useWorkspaceTasksStore();
    await store.load('ws', { assignee: 'me' }, false, WORKSPACE_ID);
    await store.load('ws', { assignee: 'me' }, true, WORKSPACE_ID);

    expect(store.tasks).toEqual([]);
    expect(store.hasData).toBe(false);
    expect(entries.size).toBe(0);

    GET.mockResolvedValueOnce({
      data: { items: [task('fresh')], has_more: false, next_cursor: null },
      error: undefined,
    });
    await store.load('ws', { assignee: 'me' }, false, WORKSPACE_ID);

    expect(store.tasks.map((item) => item.id)).toEqual(['fresh']);
  });

  it('does not retain a former query while a different exact query fails', async () => {
    GET.mockResolvedValueOnce({
      data: { items: [task('mine')], has_more: false, next_cursor: null },
      error: undefined,
    });
    GET.mockResolvedValueOnce({ data: undefined, error: { hint: 'offline' } });

    const store = useWorkspaceTasksStore();
    await store.load('ws', { assignee: 'me' });
    await store.load('ws', { sort: 'title_asc' });

    expect(store.tasks).toEqual([]);
    expect(store.hasData).toBe(false);
    expect(store.error).toBe('offline');
  });

  it('does not reuse a same-slug page after the principal changes', async () => {
    GET.mockResolvedValueOnce({
      data: { items: [task('former-principal')], has_more: false, next_cursor: null },
      error: undefined,
    });
    GET.mockResolvedValueOnce({
      data: { items: [task('current-principal')], has_more: false, next_cursor: null },
      error: undefined,
    });

    const store = useWorkspaceTasksStore();
    await store.load('ws', { assignee: 'me' }, false, WORKSPACE_ID);

    setResourceCachePrincipal('user:019ef171-bbcf-7b90-9be6-5dbb382afd09');
    await store.load('ws', { assignee: 'me' }, false, WORKSPACE_ID);

    expect(GET).toHaveBeenCalledTimes(2);
    expect(store.tasks.map((item) => item.id)).toEqual(['current-principal']);
  });

  it('does not publish a late same-slug page after a workspace transition', async () => {
    const firstResponse = deferred<{
      data: { items: TaskSummaryDto[]; has_more: false; next_cursor: null };
      error: undefined;
    }>();
    GET.mockReturnValueOnce(firstResponse.promise).mockResolvedValueOnce({
      data: { items: [task('workspace-b')], has_more: false, next_cursor: null },
      error: undefined,
    });

    const store = useWorkspaceTasksStore();
    const firstLoad = store.load('ws', { assignee: 'me' }, false, WORKSPACE_ID);
    await store.load('ws', { assignee: 'me' }, false, '019ef171-bbcf-7b90-9be6-5dbb382afd10');

    firstResponse.resolve({
      data: { items: [task('workspace-a')], has_more: false, next_cursor: null },
      error: undefined,
    });
    await firstLoad;

    expect(store.tasks.map((item) => item.id)).toEqual(['workspace-b']);
  });

  it('persists both readable and UUID task tags from the loaded page', async () => {
    const entries = new Map<string, CacheEnvelope<unknown>>();
    configureResourceCacheForTest(new ResourceCache({ store: cacheStore(entries) }));
    allowResourceCache();
    const taskId = '019ef171-bbcf-7b90-9be6-5dbb382afd09';
    GET.mockResolvedValue({
      data: { items: [task(taskId)], has_more: false, next_cursor: null },
      error: undefined,
    });

    const store = useWorkspaceTasksStore();
    await store.load('ws', {}, false, WORKSPACE_ID);

    expect([...entries.values()][0]?.tags).toEqual(
      expect.arrayContaining(['workspace-tasks', `task:ATL-${taskId}`, `task-uuid:${taskId}`]),
    );
  });

  it('evicts every cached workspace query after a task mutation invalidation', async () => {
    const entries = new Map<string, CacheEnvelope<unknown>>();
    configureResourceCacheForTest(new ResourceCache({ store: cacheStore(entries) }));
    allowResourceCache();
    GET.mockResolvedValue({
      data: { items: [task('cached')], has_more: false, next_cursor: null },
      error: undefined,
    });

    const store = useWorkspaceTasksStore();
    await store.load('ws', { assignee: 'me' }, false, WORKSPACE_ID);
    await store.load('ws', { priority: 'high' }, false, WORKSPACE_ID);

    expect(entries.size).toBe(2);
    await expect(store.invalidateCachedQueries(WORKSPACE_ID)).resolves.toBe(true);
    expect(entries.size).toBe(0);
  });

  it('keeps the latest saved view when responses resolve out of order', async () => {
    const firstResponse = deferred<{
      data: { items: TaskSummaryDto[]; has_more: false; next_cursor: null };
      error: undefined;
    }>();
    const secondResponse = deferred<{
      data: { items: TaskSummaryDto[]; has_more: false; next_cursor: null };
      error: undefined;
    }>();
    GET.mockReturnValueOnce(firstResponse.promise).mockReturnValueOnce(secondResponse.promise);

    const store = useWorkspaceTasksStore();
    const firstLoad = store.load('ws', { assignee: 'me' });
    const secondLoad = store.load('ws', { sort: 'updated_at_desc' });

    secondResponse.resolve({
      data: { items: [task('2')], has_more: false, next_cursor: null },
      error: undefined,
    });
    await secondLoad;
    firstResponse.resolve({
      data: { items: [task('1')], has_more: false, next_cursor: null },
      error: undefined,
    });
    await firstLoad;

    expect(store.loading).toBe(false);
    expect(store.tasks.map((item) => item.id)).toEqual(['2']);
  });

  it('invalidates a pending view load when navigation returns to a cached view', async () => {
    GET.mockResolvedValueOnce({
      data: { items: [task('1')], has_more: false, next_cursor: null },
      error: undefined,
    });
    const pendingResponse = deferred<{
      data: { items: TaskSummaryDto[]; has_more: false; next_cursor: null };
      error: undefined;
    }>();
    GET.mockReturnValueOnce(pendingResponse.promise);

    const store = useWorkspaceTasksStore();
    await store.load('ws', { assignee: 'me' }, false, WORKSPACE_ID);
    const pendingLoad = store.load('ws', { sort: 'updated_at_desc' }, false, WORKSPACE_ID);

    await store.load('ws', { assignee: 'me' }, false, WORKSPACE_ID);
    expect(store.loading).toBe(false);
    expect(store.tasks.map((item) => item.id)).toEqual(['1']);

    pendingResponse.resolve({
      data: { items: [task('2')], has_more: false, next_cursor: null },
      error: undefined,
    });
    await pendingLoad;

    expect(store.tasks.map((item) => item.id)).toEqual(['1']);
  });

  it('moves periodic ownership from B back to remembered A without publishing B late', async () => {
    let now = 0;
    const scheduled: Array<{ callback: () => void; delay: number }> = [];
    const timer = {
      clear: vi.fn(),
      schedule: vi.fn((delay: number, callback: () => void) => {
        scheduled.push({ delay, callback });
        return scheduled.length;
      }),
    };
    const cache = new ResourceCache({
      store: cacheStore(new Map()),
      clock: { now: () => now },
      timer,
    });
    configureResourceCacheForTest(cache);
    allowResourceCache();
    const bResponse = deferred<{
      data: { items: TaskSummaryDto[]; has_more: false; next_cursor: null };
      error: undefined;
    }>();
    GET.mockResolvedValueOnce({
      data: { items: [task('a')], has_more: false, next_cursor: null },
      error: undefined,
    });
    GET.mockReturnValueOnce(bResponse.promise).mockResolvedValueOnce({
      data: { items: [task('a-refreshed')], has_more: false, next_cursor: null },
      error: undefined,
    });

    const store = useWorkspaceTasksStore();
    await store.load('ws', { assignee: 'me' }, false, WORKSPACE_ID);
    const loadB = store.load('ws', { sort: 'updated_at_desc' }, false, WORKSPACE_ID);
    await store.load('ws', { assignee: 'me' }, false, WORKSPACE_ID);

    bResponse.resolve({
      data: { items: [task('b-late')], has_more: false, next_cursor: null },
      error: undefined,
    });
    await loadB;

    now = 60_000;
    scheduled.at(-1)?.callback();
    await vi.waitFor(() => expect(store.tasks.map((item) => item.id)).toEqual(['a-refreshed']));

    expect(GET).toHaveBeenLastCalledWith('/api/workspaces/{ws}/tasks', {
      params: { path: { ws: 'ws' }, query: { assignee: 'me', limit: 200 } },
    });
  });

  it('starts the initial network request while an empty cache lookup is pending', async () => {
    let resolveCache: (() => void) | undefined;
    const store = cacheStore(new Map());
    store.get = vi.fn(
      <T>(_key: string, _schema: ZodType<T>) =>
        new Promise<CacheEnvelope<T> | null>((resolve) => {
          resolveCache = () => resolve(null);
        }),
    ) as ResourceCacheStore['get'];
    configureResourceCacheForTest(new ResourceCache({ store }));
    allowResourceCache();
    const response = deferred<{
      data: { items: TaskSummaryDto[]; has_more: false; next_cursor: null };
      error: undefined;
    }>();
    GET.mockReturnValueOnce(response.promise);

    const storeUnderTest = useWorkspaceTasksStore();
    const load = storeUnderTest.load('ws', { assignee: 'me' }, false, WORKSPACE_ID);

    expect(storeUnderTest.loading).toBe(true);
    expect(GET).toHaveBeenCalledOnce();

    resolveCache?.();
    response.resolve({
      data: { items: [task('1')], has_more: false, next_cursor: null },
      error: undefined,
    });
    await load;
  });

  it('publishes success and settles loading without waiting for hung hydration', async () => {
    const store = cacheStore(new Map());
    store.get = vi.fn(() => new Promise(() => undefined)) as ResourceCacheStore['get'];
    configureResourceCacheForTest(new ResourceCache({ store }));
    allowResourceCache();
    GET.mockResolvedValueOnce({
      data: { items: [task('fresh')], has_more: false, next_cursor: null },
      error: undefined,
    });

    const workspaceTasks = useWorkspaceTasksStore();
    await workspaceTasks.load('ws', { assignee: 'me' }, false, WORKSPACE_ID);

    expect(workspaceTasks.loading).toBe(false);
    expect(workspaceTasks.tasks.map((item) => item.id)).toEqual(['fresh']);
    expect(GET).toHaveBeenCalledOnce();
  });

  it('settles a transport failure without waiting for hung hydration', async () => {
    const store = cacheStore(new Map());
    store.get = vi.fn(() => new Promise(() => undefined)) as ResourceCacheStore['get'];
    configureResourceCacheForTest(new ResourceCache({ store }));
    allowResourceCache();
    GET.mockRejectedValueOnce(new Error('network unavailable'));

    const workspaceTasks = useWorkspaceTasksStore();
    await workspaceTasks.load('ws', { assignee: 'me' }, false, WORKSPACE_ID);

    expect(workspaceTasks.loading).toBe(false);
    expect(workspaceTasks.tasks).toEqual([]);
    expect(workspaceTasks.error).toBe('Failed to load tasks');
    expect(GET).toHaveBeenCalledOnce();
  });

  it('accepts slow cached data after a transient failure has already settled loading', async () => {
    let resolveCache: ((entry: CacheEnvelope<unknown>) => void) | undefined;
    const store = cacheStore(new Map());
    store.get = vi.fn(
      <T>(_key: string, schema: ZodType<T>) =>
        new Promise<CacheEnvelope<T> | null>((resolve) => {
          resolveCache = (entry) => resolve({ ...entry, payload: schema.parse(entry.payload) });
        }),
    ) as ResourceCacheStore['get'];
    configureResourceCacheForTest(new ResourceCache({ store }));
    allowResourceCache();
    GET.mockRejectedValueOnce(new Error('network unavailable'));
    const workspaceTasks = useWorkspaceTasksStore();

    await workspaceTasks.load('ws', { assignee: 'me' }, false, WORKSPACE_ID);

    expect(workspaceTasks.loading).toBe(false);
    expect(workspaceTasks.tasks).toEqual([]);
    expect(workspaceTasks.error).toBe('Failed to load tasks');

    const key = buildCacheKey({
      principal: PRINCIPAL,
      workspaceId: WORKSPACE_ID,
      resourceKind: 'task-list',
      resourceId: 'workspace-tasks',
      query: { assignee: 'me', limit: 200 },
      setValuedQueryKeys: ['column_id', 'label', 'priority'],
    });
    if (key === null) throw new Error('Expected a canonical workspace task cache key');
    const now = Date.now();
    resolveCache?.({
      schema: 1,
      key,
      payloadVersion: 1,
      storedAt: now,
      validatedAt: now,
      lastAccessedAt: now,
      retentionExpiresAt: now + 60_000,
      bytes: 128,
      stale: false,
      tags: ['workspace-tasks'],
      payload: { items: [task('cached')], has_more: false, next_cursor: null },
    });
    await vi.waitFor(() => expect(workspaceTasks.tasks.map((item) => item.id)).toEqual(['cached']));

    expect(workspaceTasks.loading).toBe(false);
    expect(workspaceTasks.error).toBe('Failed to load tasks');
    expect(GET).toHaveBeenCalledOnce();
  });

  it('settles a denial before hung hydration and suppresses its late cached page', async () => {
    let resolveCache: ((entry: CacheEnvelope<unknown>) => void) | undefined;
    const store = cacheStore(new Map());
    store.get = vi.fn(
      <T>(_key: string, schema: ZodType<T>) =>
        new Promise<CacheEnvelope<T> | null>((resolve) => {
          resolveCache = (entry) => resolve({ ...entry, payload: schema.parse(entry.payload) });
        }),
    ) as ResourceCacheStore['get'];
    configureResourceCacheForTest(new ResourceCache({ store }));
    allowResourceCache();
    GET.mockResolvedValueOnce({ data: undefined, error: { status: 403, hint: 'Denied' } });
    const workspaceTasks = useWorkspaceTasksStore();

    await workspaceTasks.load('ws', { assignee: 'me' }, false, WORKSPACE_ID);

    expect(workspaceTasks.loading).toBe(false);
    expect(workspaceTasks.tasks).toEqual([]);
    expect(workspaceTasks.error).toBe('Denied');

    const key = buildCacheKey({
      principal: PRINCIPAL,
      workspaceId: WORKSPACE_ID,
      resourceKind: 'task-list',
      resourceId: 'workspace-tasks',
      query: { assignee: 'me', limit: 200 },
      setValuedQueryKeys: ['column_id', 'label', 'priority'],
    });
    if (key === null) throw new Error('Expected a canonical workspace task cache key');
    const now = Date.now();
    resolveCache?.({
      schema: 1,
      key,
      payloadVersion: 1,
      storedAt: now,
      validatedAt: now,
      lastAccessedAt: now,
      retentionExpiresAt: now + 60_000,
      bytes: 128,
      stale: false,
      tags: ['workspace-tasks'],
      payload: { items: [task('denied-cache')], has_more: false, next_cursor: null },
    });
    await Promise.resolve();

    expect(workspaceTasks.tasks).toEqual([]);
  });

  it('settles transport failures without leaving the saved view loading', async () => {
    GET.mockRejectedValueOnce(new Error('network unavailable'));

    const store = useWorkspaceTasksStore();
    await expect(store.load('ws', { assignee: 'me' })).resolves.toBe(true);

    expect(store.loading).toBe(false);
    expect(store.error).toBe('Failed to load tasks');
  });
});
