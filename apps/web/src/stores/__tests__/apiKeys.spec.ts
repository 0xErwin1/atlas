import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET, PATCH, POST } = vi.hoisted(() => ({ GET: vi.fn(), PATCH: vi.fn(), POST: vi.fn() }));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET, PATCH, POST },
}));

import { type ApiKeyDto, useApiKeysStore } from '@/stores/apiKeys';

function key(over: Partial<ApiKeyDto> = {}): ApiKeyDto {
  return {
    id: 'k1',
    name: 'ci-bot',
    type: 'agent',
    key_kind: 'personal',
    created_at: '2024-01-01T00:00:00Z',
    is_global: false,
    scopes: [],
    ...over,
  } as ApiKeyDto;
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
    expect(PATCH).toHaveBeenCalledWith('/api/v2/custos/personal-api-keys/{key_id}', {
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

describe('useApiKeysStore — setKeyScopes', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('PATCHes the scope set and reflects the server value on success', async () => {
    PATCH.mockResolvedValueOnce({
      data: key({ scopes: ['acta::tasks::read', 'acta::tasks::create'] }),
      error: undefined,
    });

    const store = useApiKeysStore();
    store.keys = [key({ scopes: [] })];

    const ok = await store.setKeyScopes('k1', ['acta::tasks::read', 'acta::tasks::create']);

    expect(ok).toBe(true);
    expect(PATCH).toHaveBeenCalledWith('/api/v2/custos/personal-api-keys/{key_id}', {
      params: { path: { key_id: 'k1' } },
      body: { scopes: ['acta::tasks::read', 'acta::tasks::create'] },
    });
    expect(store.keys[0]?.scopes).toEqual(['acta::tasks::read', 'acta::tasks::create']);
    expect(store.error).toBeNull();
  });

  it('sets the error and returns false on failure, leaving local scopes unchanged', async () => {
    PATCH.mockResolvedValueOnce({
      data: undefined,
      error: { hint: 'Not allowed' },
    });

    const store = useApiKeysStore();
    store.keys = [key({ scopes: ['acta::tasks::read'] })];

    const ok = await store.setKeyScopes('k1', ['acta::docs::delete']);

    expect(ok).toBe(false);
    expect(store.error).toBe('Not allowed');
    expect(store.keys[0]?.scopes).toEqual(['acta::tasks::read']);
  });
});

describe('useApiKeysStore — setKeyWorkspaceRole', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('POSTs the api_key principal grant to the right workspace and returns true', async () => {
    POST.mockResolvedValueOnce({ data: {}, error: undefined });

    const store = useApiKeysStore();
    const ok = await store.setKeyWorkspaceRole('k1', 'beta', 'editor');

    expect(ok).toBe(true);
    expect(POST).toHaveBeenCalledWith('/api/v2/custos/workspaces/{ws}/grants', {
      params: { path: { ws: 'beta' } },
      body: { principal: { type: 'api_key', id: 'k1' }, role: 'editor' },
    });
    expect(store.error).toBeNull();
  });

  it('sets the error and returns false on failure', async () => {
    POST.mockResolvedValueOnce({ data: undefined, error: { hint: 'Forbidden' } });

    const store = useApiKeysStore();
    const ok = await store.setKeyWorkspaceRole('k1', 'beta', 'editor');

    expect(ok).toBe(false);
    expect(store.error).toBe('Forbidden');
  });
});

describe('useApiKeysStore — the two key families (v2-e4-s3b)', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('creates a personal key through the personal family path', async () => {
    POST.mockResolvedValueOnce({ data: { id: 'k9', secret: 'atlas_pk_x' }, error: undefined });

    const store = useApiKeysStore();
    const created = await store.createKey({
      name: 'mine',
      type: null,
      expires_at: null,
      scopes: null,
      initial_grant: null,
    });

    expect(created?.id).toBe('k9');
    expect(POST).toHaveBeenCalledWith('/api/v2/custos/personal-api-keys', {
      body: { name: 'mine', type: null, expires_at: null, scopes: null, initial_grant: null },
    });
  });

  it('addresses an agent-kind key through the agent family path', async () => {
    PATCH.mockResolvedValueOnce({ data: key({ key_kind: 'agent', is_global: true }), error: undefined });

    const store = useApiKeysStore();
    store.keys = [key({ key_kind: 'agent', is_global: false })];

    const ok = await store.setKeyGlobal('k1', true);

    expect(ok).toBe(true);
    expect(PATCH).toHaveBeenCalledWith('/api/v2/custos/agent-api-keys/{key_id}', {
      params: { path: { key_id: 'k1' } },
      body: { is_global: true },
    });
  });

  it('loads both families and merges the non-revoked keys', async () => {
    GET.mockImplementation((path: string) => {
      if (path === '/api/v2/custos/personal-api-keys') {
        return Promise.resolve({
          data: { items: [key({ id: 'p1', key_kind: 'personal' })], next_cursor: undefined, has_more: false },
          error: undefined,
        });
      }
      if (path === '/api/v2/custos/agent-api-keys') {
        return Promise.resolve({
          data: {
            items: [
              key({ id: 'a1', key_kind: 'agent' }),
              key({ id: 'a2', key_kind: 'agent', revoked_at: '2024-02-01T00:00:00Z' }),
            ],
            next_cursor: undefined,
            has_more: false,
          },
          error: undefined,
        });
      }
      return Promise.reject(new Error(`unexpected path ${path}`));
    });

    const store = useApiKeysStore();
    await store.loadKeys();

    expect(GET).toHaveBeenCalledWith('/api/v2/custos/personal-api-keys', expect.anything());
    expect(GET).toHaveBeenCalledWith('/api/v2/custos/agent-api-keys', expect.anything());
    expect(store.keys.map((k) => k.id).sort()).toEqual(['a1', 'p1']);
    expect(store.error).toBeNull();
  });
});
