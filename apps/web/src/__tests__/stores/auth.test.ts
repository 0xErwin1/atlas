import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { platformTransport } = vi.hoisted(() => ({
  platformTransport: {
    login: vi.fn(),
    me: vi.fn(),
    resume: vi.fn(),
    logout: vi.fn(),
    createWorkspaceEventSource: vi.fn(),
  },
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: {
    GET: vi.fn(),
    POST: vi.fn(),
  },
}));

vi.mock('@/platform/transport', () => ({
  getPlatformTransport: () => platformTransport,
}));

vi.mock('@/lib/workspaceLiveUpdates', () => ({
  disposeWorkspaceLiveUpdates: vi.fn(),
  setWorkspaceLiveUpdatesAuthorizationInvalidator: vi.fn(),
}));

vi.mock('@/cache/cacheRuntime', () => ({
  allowResourceCache: vi.fn(),
  blockAndPurgeResourceCache: vi.fn().mockResolvedValue(true),
  setResourceCachePrincipal: vi.fn(),
}));

import { wrappedClient } from '@/api/wrapper';
import {
  allowResourceCache,
  blockAndPurgeResourceCache,
  setResourceCachePrincipal,
} from '@/cache/cacheRuntime';
import {
  disposeWorkspaceLiveUpdates,
  setWorkspaceLiveUpdatesAuthorizationInvalidator,
} from '@/lib/workspaceLiveUpdates';
import { type MeResponse, useAuthStore } from '@/stores/auth';

const mockGet = wrappedClient.GET as ReturnType<typeof vi.fn>;
const mockPost = wrappedClient.POST as ReturnType<typeof vi.fn>;

const meOk = (username: string, id = '019ef171-bbcf-7b90-9be6-5dbb382afd08') =>
  Promise.resolve({
    data: {
      agent: null,
      id,
      is_root: false,
      is_system_admin: false,
      principal_type: 'user',
      username,
    } satisfies MeResponse,
    error: undefined,
  });

const apiKeyMe = (username: string, agentId = '019ef171-bbcf-7b90-9be6-5dbb382afd08') =>
  Promise.resolve({
    data: {
      agent: { id: agentId, name: username, scopes: [] },
      id: null,
      is_root: false,
      is_system_admin: false,
      principal_type: 'api_key',
      username,
    } satisfies MeResponse,
    error: undefined,
  });

const meErr = () =>
  Promise.resolve({
    data: undefined,
    error: { type: 'urn:atlas:error:auth-failed', title: 'Unauthorized', status: 401 },
  });

const postOk = () => Promise.resolve({ data: {}, error: undefined });

const postErr = (status = 401) =>
  Promise.resolve({
    data: undefined,
    error: { type: 'urn:atlas:error:auth-failed', title: 'Invalid credentials', status },
  });

describe('useAuthStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    platformTransport.login.mockImplementation((credentials: { username: string; password: string }) =>
      wrappedClient.POST('/api/auth/login', { body: credentials }),
    );
    platformTransport.me.mockImplementation(() => wrappedClient.GET('/api/auth/me', {}));
    platformTransport.resume.mockImplementation(() => platformTransport.me());
    platformTransport.logout.mockImplementation(() => wrappedClient.POST('/api/auth/logout', {}));
  });

  it('uses the platform transport seam for login, identity, and logout without changing failures', async () => {
    platformTransport.login.mockResolvedValueOnce({ data: {}, error: undefined });
    platformTransport.me.mockReturnValueOnce(meOk('desktop-user'));
    platformTransport.logout.mockResolvedValueOnce({ data: {}, error: undefined });

    const store = useAuthStore();
    const result = await store.login({ username: 'desktop-user', password: 'pass' });
    await store.logout();

    expect(result).toEqual({ ok: true });
    expect(platformTransport.login).toHaveBeenCalledExactlyOnceWith({
      username: 'desktop-user',
      password: 'pass',
    });
    expect(platformTransport.me).toHaveBeenCalledOnce();
    expect(platformTransport.logout).toHaveBeenCalledOnce();
    expect(store.isAuthenticated).toBe(false);
  });

  it('starts unauthenticated with null user', () => {
    const store = useAuthStore();
    expect(store.isAuthenticated).toBe(false);
    expect(store.user).toBeNull();
  });

  it('initializes desktop authentication through the typed resume handshake before hydrating identity', async () => {
    platformTransport.resume.mockReturnValueOnce(meOk('resumed-user'));
    const store = useAuthStore();

    await store.initialize();

    expect(platformTransport.resume).toHaveBeenCalledOnce();
    expect(platformTransport.me).not.toHaveBeenCalled();
    expect(store.isAuthenticated).toBe(true);
    expect(store.user?.username).toBe('resumed-user');
  });

  it('keeps startup unauthenticated when the typed resume handshake reports credential loss', async () => {
    platformTransport.resume.mockResolvedValueOnce({
      data: undefined,
      error: { type: 'urn:atlas:error:auth-failed', title: 'Unauthorized', status: 401 },
    });
    const store = useAuthStore();

    await store.initialize();

    expect(platformTransport.resume).toHaveBeenCalledOnce();
    expect(store.isAuthenticated).toBe(false);
    expect(store.user).toBeNull();
  });

  it('fetchMe 401 sets isAuthenticated false (REQ-W8)', async () => {
    mockGet.mockReturnValueOnce(meErr());

    const store = useAuthStore();
    await store.fetchMe();

    expect(store.isAuthenticated).toBe(false);
    expect(store.user).toBeNull();
  });

  it('disposes live updates before clearing authentication state', () => {
    const store = useAuthStore();
    store.user = { username: 'alice' } as MeResponse;
    store.isAuthenticated = true;

    store.clearUser();

    expect(blockAndPurgeResourceCache).toHaveBeenCalledOnce();
    expect(disposeWorkspaceLiveUpdates).toHaveBeenCalledOnce();
    expect(store.user).toBeNull();
    expect(store.isAuthenticated).toBe(false);
  });

  it('registers the default broker invalidator through clearUser', () => {
    const store = useAuthStore();
    store.user = { username: 'alice' } as MeResponse;
    store.isAuthenticated = true;

    const invalidate = vi.mocked(setWorkspaceLiveUpdatesAuthorizationInvalidator).mock.calls[0]?.[0];
    invalidate?.();

    expect(disposeWorkspaceLiveUpdates).toHaveBeenCalledOnce();
    expect(store.isAuthenticated).toBe(false);
  });

  it('consumes a desktop scoped auth-loss action by clearing the active Vue session', () => {
    const store = useAuthStore();
    store.user = { username: 'alice' } as MeResponse;
    store.isAuthenticated = true;

    window.dispatchEvent(
      new CustomEvent('atlas:session-action', {
        detail: { origin: 'https://atlas.iperez.dev', identity: 'user-1', cancel_transport: true },
      }),
    );

    expect(store.isAuthenticated).toBe(false);
    expect(store.user).toBeNull();
    expect(blockAndPurgeResourceCache).toHaveBeenCalledOnce();
  });

  it('consumes an origin-scoped desktop auth-loss action when the keyring identity is unavailable', () => {
    const store = useAuthStore();
    store.user = { username: 'alice' } as MeResponse;
    store.isAuthenticated = true;

    window.dispatchEvent(
      new CustomEvent('atlas:session-action', {
        detail: { origin: 'https://atlas.iperez.dev', identity: null, cancel_transport: true },
      }),
    );

    expect(store.isAuthenticated).toBe(false);
    expect(store.user).toBeNull();
    expect(blockAndPurgeResourceCache).toHaveBeenCalledOnce();
  });

  it('fetchMe 200 hydrates user and sets isAuthenticated true (REQ-W8)', async () => {
    mockGet.mockReturnValueOnce(meOk('alice'));

    const store = useAuthStore();
    await store.fetchMe();

    expect(store.isAuthenticated).toBe(true);
    expect(store.user?.username).toBe('alice');
    expect(store.apiKeyWarning).toBe(false);
    expect(allowResourceCache).toHaveBeenCalledOnce();
    expect(setResourceCachePrincipal).toHaveBeenCalledWith('user:019ef171-bbcf-7b90-9be6-5dbb382afd08');
  });

  it('purges and blocks the prior principal before accepting a different /auth/me identity', async () => {
    const store = useAuthStore();
    store.user = {
      id: '019ef171-bbcf-7b90-9be6-5dbb382afd09',
      principal_type: 'user',
      username: 'prior',
    } as MeResponse;
    store.isAuthenticated = true;
    mockGet.mockReturnValueOnce(meOk('current'));

    await store.fetchMe();

    expect(blockAndPurgeResourceCache).toHaveBeenCalledOnce();
    expect(setResourceCachePrincipal).toHaveBeenNthCalledWith(1, undefined);
    expect(setResourceCachePrincipal).toHaveBeenLastCalledWith('user:019ef171-bbcf-7b90-9be6-5dbb382afd08');
    expect(store.user?.username).toBe('current');
  });

  it('does not accept a recovered identity when the prior principal purge fails', async () => {
    const store = useAuthStore();
    store.user = {
      id: '019ef171-bbcf-7b90-9be6-5dbb382afd09',
      principal_type: 'user',
      username: 'prior',
    } as MeResponse;
    store.isAuthenticated = true;
    vi.mocked(blockAndPurgeResourceCache).mockResolvedValueOnce(false);
    mockGet.mockReturnValueOnce(meOk('current'));

    await store.fetchMe();

    expect(store.isAuthenticated).toBe(false);
    expect(store.user).toBeNull();
    expect(allowResourceCache).not.toHaveBeenCalled();
  });

  it('does not let a response from before clearUser revive a prior session over a newer fetch', async () => {
    let resolvePrior: ((value: Awaited<ReturnType<typeof meOk>>) => void) | undefined;
    mockGet.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolvePrior = resolve;
        }),
    );
    mockGet.mockReturnValueOnce(meOk('new-session'));
    const store = useAuthStore();

    const priorFetch = store.fetchMe();
    await store.clearUser();
    await store.fetchMe();
    resolvePrior?.(await meOk('prior-session'));
    await priorFetch;

    expect(store.isAuthenticated).toBe(true);
    expect(store.user?.username).toBe('new-session');
  });

  it('forgets the cache principal before beginning the logout purge', () => {
    const store = useAuthStore();
    store.user = { username: 'alice' } as MeResponse;
    store.isAuthenticated = true;

    store.clearUser();

    expect(setResourceCachePrincipal).toHaveBeenCalledWith(undefined);
    expect(blockAndPurgeResourceCache).toHaveBeenCalledOnce();
  });

  it('api_key principal_type sets apiKeyWarning (REQ-W8)', async () => {
    mockGet.mockReturnValueOnce(apiKeyMe('agent'));

    const store = useAuthStore();
    await store.fetchMe();

    expect(store.apiKeyWarning).toBe(true);
    expect(setResourceCachePrincipal).toHaveBeenCalledWith('api_key:019ef171-bbcf-7b90-9be6-5dbb382afd08');
  });

  it('purges an API-key session before accepting a different recovered API-key identity', async () => {
    const store = useAuthStore();
    mockGet.mockReturnValueOnce(apiKeyMe('first', '019ef171-bbcf-7b90-9be6-5dbb382afd08'));
    await store.fetchMe();
    mockGet.mockReturnValueOnce(apiKeyMe('second', '019ef171-bbcf-7b90-9be6-5dbb382afd09'));

    await store.fetchMe();

    expect(blockAndPurgeResourceCache).toHaveBeenCalledOnce();
    expect(setResourceCachePrincipal).toHaveBeenNthCalledWith(2, undefined);
    expect(setResourceCachePrincipal).toHaveBeenLastCalledWith(
      'api_key:019ef171-bbcf-7b90-9be6-5dbb382afd09',
    );
    expect(store.user?.agent?.id).toBe('019ef171-bbcf-7b90-9be6-5dbb382afd09');
  });

  it('fails closed when an API-key /auth/me response omits its agent identity', async () => {
    mockGet.mockReturnValueOnce(
      Promise.resolve({
        data: {
          agent: null,
          id: null,
          is_root: false,
          is_system_admin: false,
          principal_type: 'api_key',
          username: 'agent',
        } satisfies MeResponse,
        error: undefined,
      }),
    );

    const store = useAuthStore();
    await store.fetchMe();

    expect(store.isAuthenticated).toBe(false);
    expect(store.user).toBeNull();
    expect(allowResourceCache).not.toHaveBeenCalled();
    expect(setResourceCachePrincipal).toHaveBeenCalledWith(undefined);
  });

  it('logout clears store even when POST fails (REQ-W9)', async () => {
    mockGet.mockReturnValueOnce(meOk('alice'));
    await useAuthStore().fetchMe();

    mockPost.mockRejectedValueOnce(new Error('network error'));

    const store = useAuthStore();
    await store.logout();

    expect(store.isAuthenticated).toBe(false);
    expect(store.user).toBeNull();
  });

  it('blocks and hides synchronously, then awaits the purge before logout waits on the server', async () => {
    let resolvePurge: (() => void) | undefined;
    let resolveLogout: (() => void) | undefined;
    vi.mocked(blockAndPurgeResourceCache).mockImplementationOnce(
      () =>
        new Promise<boolean>((resolve) => {
          resolvePurge = () => resolve(true);
        }),
    );
    mockGet.mockReturnValueOnce(meOk('alice'));
    await useAuthStore().fetchMe();
    mockPost.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveLogout = () => resolve({ data: {}, error: undefined });
        }),
    );

    const store = useAuthStore();
    const logout = store.logout();

    expect(store.isAuthenticated).toBe(false);
    expect(store.user).toBeNull();
    expect(mockPost).not.toHaveBeenCalled();

    resolvePurge?.();
    await vi.waitFor(() => expect(mockPost).toHaveBeenCalledOnce());
    resolveLogout?.();
    await logout;
  });

  it('still revokes the session through the transport when the global cache purge fails', async () => {
    mockGet.mockReturnValueOnce(meOk('alice'));
    vi.mocked(blockAndPurgeResourceCache).mockResolvedValueOnce(false);
    await useAuthStore().fetchMe();

    await useAuthStore().logout();

    expect(platformTransport.logout).toHaveBeenCalledOnce();
    expect(useAuthStore().isAuthenticated).toBe(false);
  });

  it('login returns ok on 200', async () => {
    mockPost.mockReturnValueOnce(postOk());
    mockGet.mockReturnValueOnce(meOk('bob'));

    const store = useAuthStore();
    const result = await store.login({ username: 'bob', password: 'pass' });

    expect(result.ok).toBe(true);
    expect(store.isAuthenticated).toBe(true);
  });

  it('login returns problem on failure', async () => {
    mockPost.mockReturnValueOnce(postErr(401));

    const store = useAuthStore();
    const result = await store.login({ username: 'bad', password: 'wrong' });

    expect(result.ok).toBe(false);
    expect(store.isAuthenticated).toBe(false);
  });

  it('login surfaces an unreachable problem when the request throws (server down)', async () => {
    mockPost.mockRejectedValueOnce(new Error('Failed to fetch'));

    const store = useAuthStore();
    const result = await store.login({ username: 'bob', password: 'pass' });

    expect(result.ok).toBe(false);
    expect(result.problem?.type).toBe('urn:atlas:error:unreachable');
    expect(result.problem?.hint).toBeTruthy();
  });

  it('login still returns a problem when the error body is empty', async () => {
    mockPost.mockReturnValueOnce(Promise.resolve({ data: undefined, error: undefined }));

    const store = useAuthStore();
    const result = await store.login({ username: 'bob', password: 'pass' });

    expect(result.ok).toBe(false);
    expect(result.problem).toBeTruthy();
  });
});
