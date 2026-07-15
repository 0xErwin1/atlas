import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET, PATCH } = vi.hoisted(() => ({
  GET: vi.fn(),
  PATCH: vi.fn(),
}));

const { cacheHydrate, cacheRevalidate, cacheActivate, cacheDeactivate, cachePurgeTags, cachePrincipal } =
  vi.hoisted(() => ({
    cacheHydrate: vi.fn(),
    cacheRevalidate: vi.fn(),
    cacheActivate: vi.fn(),
    cacheDeactivate: vi.fn(),
    cachePurgeTags: vi.fn(),
    cachePrincipal: { value: 'user:018f0000-0000-7000-8000-000000000001' },
  }));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET, PATCH },
}));

vi.mock('@/cache/cacheRuntime', () => ({
  getResourceCachePrincipal: () => cachePrincipal.value,
  resourceCacheEpoch: { __v_isRef: true, value: 0 },
  invalidateTaskCache: cachePurgeTags,
  resourceCache: {
    isAvailable: () => true,
    hydrate: cacheHydrate,
    revalidate: cacheRevalidate,
    activate: cacheActivate,
    deactivate: cacheDeactivate,
  },
}));

import { useTasksStore } from '@/stores/tasks';

const taskDto = (description = '', readableId = 'ATL-1') => ({
  id: 'uuid-1',
  readable_id: readableId,
  title: 'Test task',
  description,
  board_id: 'board-1',
  column_id: 'col-1',
  project_id: 'proj-1',
  workspace_id: 'ws-1',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
  created_by: { id: 'u1', type: 'user', display_name: 'Alice' },
});

describe('useTasksStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    cachePrincipal.value = 'user:018f0000-0000-7000-8000-000000000001';
  });

  describe('loadTask', () => {
    it('populates openTask on success', async () => {
      GET.mockResolvedValueOnce({ data: taskDto(), error: undefined });

      const store = useTasksStore();
      await store.loadTask('ws-1', 'ATL-1');

      expect(store.openTask?.readable_id).toBe('ATL-1');
      expect(store.loading).toBe(false);
      expect(store.error).toBeNull();
    });

    it('rejects a network task payload with another workspace or readable ID', async () => {
      GET.mockResolvedValueOnce({
        data: { ...taskDto('', 'ATL-2'), workspace_id: 'ws-2' },
        error: undefined,
      });

      const store = useTasksStore();
      await store.loadTask('ws-1', 'ATL-1', 'ws-1');

      expect(store.openTask).toBeNull();
      expect(store.error).toBe('Failed to load task');
    });

    it('hydrates the exact cached task before a failed refresh retains it', async () => {
      const cached = taskDto('cached task', 'ATL-1');
      cacheHydrate.mockImplementationOnce(async (request) => {
        request.publish(cached);
        return cached;
      });
      cacheRevalidate.mockRejectedValueOnce(new Error('Network unavailable'));

      const store = useTasksStore();
      await store.loadTask('ws-1', 'ATL-1', '018f0000-0000-7000-8000-000000000002');

      expect(store.openTask?.description).toBe('cached task');
      expect(store.loading).toBe(false);
      expect(store.error).toBe('Failed to load task');
      expect(cacheHydrate).toHaveBeenCalledOnce();
      expect(cacheActivate).toHaveBeenCalledOnce();
    });

    it('retracts the current task and its exact cache after deletion', async () => {
      cacheHydrate.mockImplementationOnce(async (request) => {
        request.publish(taskDto('cached task'));
        return taskDto('cached task');
      });
      cacheRevalidate.mockResolvedValueOnce(undefined);

      const store = useTasksStore();
      await store.loadTask('ws-1', 'ATL-1', '018f0000-0000-7000-8000-000000000002');
      await store.retractTask('ATL-1');

      expect(store.openTask).toBeNull();
      expect(cachePurgeTags).toHaveBeenCalledWith('018f0000-0000-7000-8000-000000000002', 'ATL-1');
    });

    it('sets error on failure', async () => {
      GET.mockResolvedValueOnce({ data: undefined, error: { hint: 'Not found' } });

      const store = useTasksStore();
      await store.loadTask('ws-1', 'ATL-99');

      expect(store.openTask).toBeNull();
      expect(store.error).toBe('Not found');
    });

    it('clears a different open task before loading the requested one', async () => {
      GET.mockResolvedValueOnce({ data: taskDto('', 'ATL-2'), error: undefined });

      const store = useTasksStore();
      store.$patch({ openTask: taskDto('', 'ATL-1') });
      const pending = store.loadTask('ws-1', 'ATL-2');

      expect(store.openTask).toBeNull();

      await pending;
      expect(store.openTask?.readable_id).toBe('ATL-2');
    });

    it('clears the current task when the same readable ID loads in another workspace', async () => {
      let resolveLoad: (value: { data: ReturnType<typeof taskDto>; error: undefined }) => void = () => {};
      GET.mockReturnValueOnce(
        new Promise((resolve) => {
          resolveLoad = resolve;
        }),
      );

      const store = useTasksStore();
      store.$patch({ openTask: taskDto('', 'ATL-1') });
      const pending = store.loadTask('ws-2', 'ATL-1');

      expect(store.openTask).toBeNull();
      expect(store.loading).toBe(true);

      resolveLoad({ data: taskDto('', 'ATL-1'), error: undefined });
      await pending;
      expect(store.openTask?.readable_id).toBe('ATL-1');
    });

    it('retracts the same route target and rejects its late completion after a principal change', async () => {
      let resolveCurrent: (value: { data: ReturnType<typeof taskDto>; error: undefined }) => void = () => {};
      GET.mockResolvedValueOnce({ data: taskDto('former principal'), error: undefined });
      GET.mockReturnValueOnce(
        new Promise((resolve) => {
          resolveCurrent = resolve;
        }),
      );

      const store = useTasksStore();
      await store.loadTask('ws-1', 'ATL-1', 'ws-1');
      cachePrincipal.value = 'user:018f0000-0000-7000-8000-000000000002';
      const current = store.loadTask('ws-1', 'ATL-1', 'ws-1');

      expect(store.openTask).toBeNull();
      resolveCurrent({ data: taskDto('current principal'), error: undefined });
      await current;

      expect(store.openTask?.description).toBe('current principal');
    });

    it('does not let an older request replace the latest requested task', async () => {
      let resolveFirst: (value: { data: ReturnType<typeof taskDto>; error: undefined }) => void = () => {};
      GET.mockReturnValueOnce(
        new Promise((resolve) => {
          resolveFirst = resolve;
        }),
      );
      GET.mockResolvedValueOnce({ data: taskDto('', 'ATL-2'), error: undefined });

      const store = useTasksStore();
      const first = store.loadTask('ws-1', 'ATL-1');
      await store.loadTask('ws-1', 'ATL-2');
      resolveFirst({ data: taskDto('', 'ATL-1'), error: undefined });
      await first;

      expect(store.openTask?.readable_id).toBe('ATL-2');
    });

    it('does not let a response from another workspace replace the current task', async () => {
      let resolveFirst: (value: { data: ReturnType<typeof taskDto>; error: undefined }) => void = () => {};
      GET.mockReturnValueOnce(
        new Promise((resolve) => {
          resolveFirst = resolve;
        }),
      );
      GET.mockResolvedValueOnce({ data: taskDto('', 'ATL-1'), error: undefined });

      const store = useTasksStore();
      const first = store.loadTask('ws-1', 'ATL-1');
      await store.loadTask('ws-2', 'ATL-1');
      resolveFirst({ data: taskDto('stale', 'ATL-1'), error: undefined });
      await first;

      expect(store.openTask?.description).toBe('');
    });

    it('keeps the current task visible during a same-target refresh', async () => {
      let resolveRefresh: (value: { data: ReturnType<typeof taskDto>; error: undefined }) => void = () => {};
      GET.mockResolvedValueOnce({ data: taskDto('visible', 'ATL-1'), error: undefined });
      GET.mockReturnValueOnce(
        new Promise((resolve) => {
          resolveRefresh = resolve;
        }),
      );

      const store = useTasksStore();
      await store.loadTask('ws-1', 'ATL-1');
      const refresh = store.loadTask('ws-1', 'ATL-1');

      expect(store.openTask?.description).toBe('visible');
      expect(store.loading).toBe(true);

      resolveRefresh({ data: taskDto('refreshed', 'ATL-1'), error: undefined });
      await refresh;
      expect(store.openTask?.description).toBe('refreshed');
    });

    it('keeps the current task when a same-target refresh returns an error', async () => {
      GET.mockResolvedValueOnce({ data: taskDto('visible', 'ATL-1'), error: undefined });
      GET.mockResolvedValueOnce({ data: undefined, error: { hint: 'Refresh failed' } });

      const store = useTasksStore();
      await store.loadTask('ws-1', 'ATL-1');
      await store.loadTask('ws-1', 'ATL-1');

      expect(store.openTask?.description).toBe('visible');
      expect(store.loading).toBe(false);
      expect(store.error).toBe('Refresh failed');
    });

    it('keeps a visible current task when its same-target refresh is rejected', async () => {
      let rejectRefresh: (reason?: unknown) => void = () => {};
      GET.mockResolvedValueOnce({ data: taskDto('visible', 'ATL-1'), error: undefined });
      GET.mockReturnValueOnce(
        new Promise((_, reject) => {
          rejectRefresh = reject;
        }),
      );

      const store = useTasksStore();
      await store.loadTask('ws-1', 'ATL-1');
      const refresh = store.loadTask('ws-1', 'ATL-1');

      expect(store.openTask?.description).toBe('visible');
      expect(store.loading).toBe(true);

      rejectRefresh(new Error('Network unavailable'));
      await expect(refresh).resolves.toBeUndefined();

      expect(store.openTask?.description).toBe('visible');
      expect(store.loading).toBe(false);
      expect(store.error).toBe('Failed to load task');
    });

    it('ignores a rejected request after another task becomes current', async () => {
      let rejectFirst: (reason?: unknown) => void = () => {};
      GET.mockReturnValueOnce(
        new Promise((_, reject) => {
          rejectFirst = reject;
        }),
      );
      GET.mockResolvedValueOnce({ data: taskDto('current', 'ATL-2'), error: undefined });

      const store = useTasksStore();
      const first = store.loadTask('ws-1', 'ATL-1');
      await store.loadTask('ws-1', 'ATL-2');
      rejectFirst(new Error('Network unavailable'));

      await expect(first).resolves.toBeUndefined();
      expect(store.openTask?.readable_id).toBe('ATL-2');
      expect(store.openTask?.description).toBe('current');
      expect(store.loading).toBe(false);
      expect(store.error).toBeNull();
    });
  });

  describe('updateDescription', () => {
    it('updates openTask and returns true on success', async () => {
      const initial = taskDto('old');
      const updated = taskDto('new description');

      GET.mockResolvedValueOnce({ data: initial, error: undefined });
      await useTasksStore().loadTask('ws-1', 'ATL-1');

      const store = useTasksStore();
      store.$patch({ openTask: initial });
      PATCH.mockResolvedValueOnce({ data: updated, error: undefined });

      const result = await store.updateDescription('ws-1', 'ATL-1', 'new description');

      expect(result).toBe(true);
      expect(store.openTask?.description).toBe('new description');
      expect(store.error).toBeNull();
    });

    it('sets error and returns false on failure', async () => {
      const store = useTasksStore();
      store.$patch({ openTask: taskDto('old') });

      PATCH.mockResolvedValueOnce({ data: undefined, error: { hint: 'Forbidden' } });

      const result = await store.updateDescription('ws-1', 'ATL-1', 'whatever');

      expect(result).toBe(false);
      expect(store.error).toBe('Forbidden');
      expect(store.openTask?.description).toBe('old');
    });

    it('passes description in the PATCH body', async () => {
      const store = useTasksStore();
      store.$patch({ openTask: taskDto() });

      PATCH.mockResolvedValueOnce({ data: taskDto('hello'), error: undefined });

      await store.updateDescription('ws-1', 'ATL-1', 'hello');

      const [, opts] = PATCH.mock.calls[0] as [string, { body: { description: string } }];
      expect(opts.body.description).toBe('hello');
    });

    it('does not overwrite newer local edits when an older save returns', async () => {
      const store = useTasksStore();
      store.$patch({ openTask: taskDto('old') });

      let resolvePatch: (value: { data: ReturnType<typeof taskDto>; error: undefined }) => void = () => {};
      PATCH.mockReturnValueOnce(
        new Promise((resolve) => {
          resolvePatch = resolve;
        }),
      );

      const pending = store.updateDescription('ws-1', 'ATL-1', 'first save');
      store.patchOpenTask({ description: 'newer local edit' });

      resolvePatch({ data: taskDto('first save'), error: undefined });
      await pending;

      expect(store.openTask?.description).toBe('newer local edit');
    });
  });

  describe('patchOpenTask', () => {
    it('merges fields into the open task', () => {
      const store = useTasksStore();
      store.$patch({ openTask: taskDto() });

      store.patchOpenTask({ priority: 'high', column_id: 'col-2' });

      expect(store.openTask?.priority).toBe('high');
      expect(store.openTask?.column_id).toBe('col-2');
      expect(store.openTask?.title).toBe('Test task');
    });

    it('is a no-op when no task is open', () => {
      const store = useTasksStore();
      store.patchOpenTask({ priority: 'low' });
      expect(store.openTask).toBeNull();
    });
  });
});
