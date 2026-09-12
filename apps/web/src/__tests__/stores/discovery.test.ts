import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { deferred } from '@/__tests__/deferred';

const authState = { sessionGeneration: 0 };

vi.mock('@/api/wrapper', () => ({
  wrappedClient: {
    GET: vi.fn(),
    POST: vi.fn(),
    PATCH: vi.fn(),
    DELETE: vi.fn(),
  },
}));

vi.mock('@/stores/auth', () => ({
  useAuthStore: () => ({
    get sessionGeneration() {
      return authState.sessionGeneration;
    },
  }),
}));

import { wrappedClient } from '@/api/wrapper';
import { useDiscoveryStore } from '@/stores/discovery';

const mockGet = wrappedClient.GET as ReturnType<typeof vi.fn>;

const metaResponse = {
  data: {
    components: [
      { stable_id: 'acta', kind: 'product', contract_version: 1, navigation_providers: ['acta.workspace'] },
      { stable_id: 'custos', kind: 'product', contract_version: 1, navigation_providers: ['custos.admin'] },
      { stable_id: 'platform', kind: 'core', contract_version: 1 },
    ],
  },
  error: undefined,
};

const discoverResponse = {
  data: {
    admin: false,
    truncated: false,
    components: [{ component: 'acta', scopes: ['acta::workspace::w1'] }],
  },
  error: undefined,
};

function mockBothEndpointsWith(
  meta: { data?: unknown; error?: unknown },
  discover: { data?: unknown; error?: unknown },
) {
  mockGet.mockImplementation((path: string) => {
    if (path === '/api/v2/platform/meta') return Promise.resolve(meta);
    if (path === '/api/v2/custos/discover') return Promise.resolve(discover);
    throw new Error(`unexpected path: ${path}`);
  });
}

describe('useDiscoveryStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    authState.sessionGeneration = 0;
  });

  it('fetches meta and discover in parallel and derives admin/truncated/availableProviderIds', async () => {
    mockBothEndpointsWith(metaResponse, discoverResponse);
    const store = useDiscoveryStore();

    await store.load();

    expect(mockGet).toHaveBeenCalledWith('/api/v2/platform/meta', {});
    expect(mockGet).toHaveBeenCalledWith('/api/v2/custos/discover', {});
    expect(store.admin).toBe(false);
    expect(store.truncated).toBe(false);
    expect(store.availableProviderIds).toEqual(new Set(['acta.workspace']));
    expect(store.error).toBeNull();
  });

  it('composes to an empty availableProviderIds set before the first load, never crashing', () => {
    const store = useDiscoveryStore();

    expect(store.availableProviderIds).toEqual(new Set());
    expect(store.admin).toBe(false);
    expect(store.truncated).toBe(false);
  });

  it('hides everything when discover reports no present components, without crashing', async () => {
    mockBothEndpointsWith(metaResponse, {
      data: { admin: false, truncated: false, components: [] },
      error: undefined,
    });
    const store = useDiscoveryStore();

    await store.load();

    expect(store.availableProviderIds).toEqual(new Set());
  });

  it('surfaces truncated through to the composable', async () => {
    mockBothEndpointsWith(metaResponse, {
      data: {
        admin: false,
        truncated: true,
        components: [{ component: 'acta', scopes: ['acta::workspace::w1'] }],
      },
      error: undefined,
    });
    const store = useDiscoveryStore();

    await store.load();

    expect(store.truncated).toBe(true);
  });

  it('sets error and leaves prior state on a failed meta fetch', async () => {
    mockBothEndpointsWith({ data: undefined, error: { hint: 'meta down' } }, discoverResponse);
    const store = useDiscoveryStore();

    await store.load();

    expect(store.error).toBe('meta down');
    expect(store.availableProviderIds).toEqual(new Set());
  });

  it('sets error on a failed discover fetch', async () => {
    mockBothEndpointsWith(metaResponse, { data: undefined, error: { hint: 'discover down' } });
    const store = useDiscoveryStore();

    await store.load();

    expect(store.error).toBe('discover down');
  });

  it('ensureLoaded fetches once per session — a second call for the same session is a no-op', async () => {
    mockBothEndpointsWith(metaResponse, discoverResponse);
    const store = useDiscoveryStore();

    await store.ensureLoaded();
    await store.ensureLoaded();

    expect(mockGet).toHaveBeenCalledTimes(2); // one meta + one discover call, not four
  });

  it('a rejecting transport (offline, aborted fetch) leaves the store in the error state and ensureLoaded still resolves', async () => {
    mockGet.mockImplementation((path: string) => {
      if (path === '/api/v2/platform/meta') return Promise.reject(new Error('network down'));
      if (path === '/api/v2/custos/discover') return Promise.resolve(discoverResponse);
      throw new Error(`unexpected path: ${path}`);
    });
    const store = useDiscoveryStore();

    await expect(store.ensureLoaded()).resolves.toBeUndefined();
    expect(store.error).toBe('Failed to load discoverable navigation');
    expect(store.availableProviderIds).toEqual(new Set());
  });

  it('blocks a refetch inside the backoff window, then allows one once it elapses — a failure is never latched for the session', async () => {
    vi.useFakeTimers();
    mockBothEndpointsWith(metaResponse, { data: undefined, error: { hint: 'discover down' } });
    const store = useDiscoveryStore();

    await store.ensureLoaded();
    expect(mockGet).toHaveBeenCalledTimes(2);
    expect(store.error).toBe('discover down');
    expect(store.availableProviderIds).toEqual(new Set());

    await store.ensureLoaded();
    expect(mockGet).toHaveBeenCalledTimes(2); // still inside the backoff window

    vi.advanceTimersByTime(5_000);
    mockBothEndpointsWith(metaResponse, discoverResponse);
    await store.ensureLoaded();
    expect(mockGet).toHaveBeenCalledTimes(4);
    expect(store.error).toBeNull();
    expect(store.availableProviderIds).toEqual(new Set(['acta.workspace']));
    vi.useRealTimers();
  });

  it('retry() clears the backoff and refetches immediately', async () => {
    mockBothEndpointsWith(metaResponse, { data: undefined, error: { hint: 'discover down' } });
    const store = useDiscoveryStore();

    await store.ensureLoaded();
    expect(mockGet).toHaveBeenCalledTimes(2);

    mockBothEndpointsWith(metaResponse, discoverResponse);
    await store.retry();
    expect(mockGet).toHaveBeenCalledTimes(4);
    expect(store.error).toBeNull();
    expect(store.availableProviderIds).toEqual(new Set(['acta.workspace']));
  });

  it('ensureLoaded dedups concurrent callers into a single in-flight fetch', async () => {
    const metaDeferred = deferred<{ data: unknown; error: unknown }>();
    mockGet.mockImplementation((path: string) => {
      if (path === '/api/v2/platform/meta') return metaDeferred.promise;
      if (path === '/api/v2/custos/discover') return Promise.resolve(discoverResponse);
      throw new Error(`unexpected path: ${path}`);
    });
    const store = useDiscoveryStore();

    const first = store.ensureLoaded();
    const second = store.ensureLoaded();
    metaDeferred.resolve(metaResponse);
    await Promise.all([first, second]);

    expect(mockGet).toHaveBeenCalledTimes(2);
  });

  it('ensureLoaded refetches after reset (the logout/login boundary)', async () => {
    mockBothEndpointsWith(metaResponse, discoverResponse);
    const store = useDiscoveryStore();

    await store.ensureLoaded();
    store.reset();
    expect(store.availableProviderIds).toEqual(new Set());

    authState.sessionGeneration += 1;
    await store.ensureLoaded();

    expect(mockGet).toHaveBeenCalledTimes(4);
    expect(store.availableProviderIds).toEqual(new Set(['acta.workspace']));
  });

  it('reset while a fetch is pending drops the stale in-flight promise so a later ensureLoaded starts a fresh fetch', async () => {
    const metaDeferred = deferred<{ data: unknown; error: unknown }>();
    mockGet.mockImplementation((path: string) => {
      if (path === '/api/v2/platform/meta') return metaDeferred.promise;
      if (path === '/api/v2/custos/discover') return Promise.resolve(discoverResponse);
      throw new Error(`unexpected path: ${path}`);
    });
    const store = useDiscoveryStore();

    const pending = store.ensureLoaded();
    store.reset();
    authState.sessionGeneration += 1;
    mockBothEndpointsWith(metaResponse, discoverResponse);
    const afterReset = store.ensureLoaded();
    metaDeferred.resolve(metaResponse);
    await Promise.all([pending, afterReset]);

    expect(mockGet).toHaveBeenCalledTimes(4); // the stale attempt plus a fresh one, not a returned stale promise
    expect(store.availableProviderIds).toEqual(new Set(['acta.workspace']));
  });
});
