import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { deferred } from '@/__tests__/deferred';

const { GET, POST } = vi.hoisted(() => ({ GET: vi.fn(), POST: vi.fn() }));

vi.mock('@/api/wrapper', () => ({ wrappedClient: { GET, POST } }));

import { useStatusTemplatesStore } from '@/stores/statusTemplates';

const template = (id: string, name: string) => ({
  id,
  name,
  color: null,
  position_key: id,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
});

describe('useStatusTemplatesStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('keeps destination templates when an earlier workspace load or mutation settles late', async () => {
    const loadA = deferred<{ data: ReturnType<typeof template>[]; error: undefined }>();
    const createA = deferred<{ data: ReturnType<typeof template>; error: undefined }>();
    GET.mockReturnValueOnce(loadA.promise).mockResolvedValueOnce({
      data: [template('b', 'Destination')],
      error: undefined,
    });
    POST.mockReturnValueOnce(createA.promise);

    const store = useStatusTemplatesStore();
    const loadingA = store.load('workspace-a');
    const creatingA = store.create('workspace-a', 'Stale');

    store.resetWorkspace();
    await store.load('workspace-b');

    loadA.resolve({ data: [template('a', 'Stale')], error: undefined });
    createA.resolve({ data: template('a-created', 'Stale mutation'), error: undefined });
    await Promise.all([loadingA, creatingA]);

    expect(store.templates.map((item) => item.id)).toEqual(['b']);
  });
});
