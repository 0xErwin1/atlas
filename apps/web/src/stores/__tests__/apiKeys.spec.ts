import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { PATCH } = vi.hoisted(() => ({ PATCH: vi.fn() }));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { PATCH },
}));

import { type ApiKeyDto, useApiKeysStore } from '@/stores/apiKeys';

function key(over: Partial<ApiKeyDto> = {}): ApiKeyDto {
  return {
    id: 'k1',
    name: 'ci-bot',
    type: 'agent',
    created_at: '2024-01-01T00:00:00Z',
    is_global: false,
    ...over,
  };
}

describe('useApiKeysStore — setKeyGlobal', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('PATCHes the right body and reflects the server value on success', async () => {
    PATCH.mockResolvedValueOnce({
      data: key({ is_global: true }),
      error: undefined,
    });

    const store = useApiKeysStore();
    store.keys = [key({ is_global: false })];

    const ok = await store.setKeyGlobal('k1', true);

    expect(ok).toBe(true);
    expect(PATCH).toHaveBeenCalledWith('/v1/api-keys/{key_id}', {
      params: { path: { key_id: 'k1' } },
      body: { is_global: true },
    });
    expect(store.keys[0]?.is_global).toBe(true);
    expect(store.error).toBeNull();
  });

  it('sets the error and returns false on failure, leaving local state unchanged', async () => {
    PATCH.mockResolvedValueOnce({
      data: undefined,
      error: { hint: 'Not allowed' },
    });

    const store = useApiKeysStore();
    store.keys = [key({ is_global: false })];

    const ok = await store.setKeyGlobal('k1', true);

    expect(ok).toBe(false);
    expect(store.error).toBe('Not allowed');
    expect(store.keys[0]?.is_global).toBe(false);
  });
});
