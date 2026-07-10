import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { deferred } from '@/__tests__/deferred';

const { GET } = vi.hoisted(() => ({ GET: vi.fn() }));

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

describe('useWorkspaceTasksStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
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
    await store.load('ws', { assignee: 'me' });
    const pendingLoad = store.load('ws', { sort: 'updated_at_desc' });

    await store.load('ws', { assignee: 'me' });
    expect(store.loading).toBe(false);
    expect(store.tasks.map((item) => item.id)).toEqual(['1']);

    pendingResponse.resolve({
      data: { items: [task('2')], has_more: false, next_cursor: null },
      error: undefined,
    });
    await pendingLoad;

    expect(store.tasks.map((item) => item.id)).toEqual(['1']);
  });

  it('settles transport failures without leaving the saved view loading', async () => {
    GET.mockRejectedValueOnce(new Error('network unavailable'));

    const store = useWorkspaceTasksStore();
    await expect(store.load('ws', { assignee: 'me' })).resolves.toBe(true);

    expect(store.loading).toBe(false);
    expect(store.error).toBe('Failed to load tasks');
  });
});
