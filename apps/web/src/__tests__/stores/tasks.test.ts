import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET, PATCH } = vi.hoisted(() => ({
  GET: vi.fn(),
  PATCH: vi.fn(),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET, PATCH },
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
