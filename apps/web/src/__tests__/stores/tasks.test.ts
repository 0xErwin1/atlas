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

const taskDto = (description = '') => ({
  id: 'uuid-1',
  readable_id: 'ATL-1',
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
  });
});
