import { afterEach, describe, expect, it, vi } from 'vitest';

const { disposeWorkspaceLiveUpdates } = vi.hoisted(() => ({
  disposeWorkspaceLiveUpdates: vi.fn(),
}));

vi.mock('@/lib/workspaceLiveUpdates', () => ({
  disposeWorkspaceLiveUpdates,
  setWorkspaceLiveUpdatesAuthorizationInvalidator: vi.fn(),
}));
vi.mock('vue', async (importOriginal) => ({
  ...(await importOriginal<typeof import('vue')>()),
  createApp: () => ({ mount: vi.fn(), use: vi.fn() }),
}));
vi.mock('pinia', async (importOriginal) => {
  const module = await importOriginal<typeof import('pinia')>();
  return { ...module, createPinia: module.createPinia };
});
vi.mock('@/router/index', () => ({ router: {} }));

import { wrappedClient } from '@/api/wrapper';
import {
  allowResourceCache,
  configureResourceCacheForTest,
  setResourceCachePrincipal,
} from '@/cache/cacheRuntime';
import { ResourceCache } from '@/cache/resourceCache';
import { appPinia, installTransportStatus, registerWorkspaceLiveUpdatesPagehide } from '@/main';
import { useResourceStatusStore } from '@/stores/resourceStatus';
import { useWorkspaceStore } from '@/stores/workspace';

describe('workspace live update page lifecycle', () => {
  afterEach(() => {
    disposeWorkspaceLiveUpdates.mockClear();
  });

  it('disposes on a non-persisted pagehide and registers the listener only once', () => {
    const cleanup = registerWorkspaceLiveUpdatesPagehide();
    registerWorkspaceLiveUpdatesPagehide();

    window.dispatchEvent(new PageTransitionEvent('pagehide', { persisted: false }));

    expect(disposeWorkspaceLiveUpdates).toHaveBeenCalledOnce();
    cleanup();
  });

  it('retains the broker for persisted bfcache pagehide events', () => {
    const cleanup = registerWorkspaceLiveUpdatesPagehide();

    window.dispatchEvent(new PageTransitionEvent('pagehide', { persisted: true }));

    expect(disposeWorkspaceLiveUpdates).not.toHaveBeenCalled();
    cleanup();
  });
});

describe('transport status lifecycle', () => {
  it('installs browser listeners once and removes them through the shared cleanup', () => {
    installTransportStatus()();
    const addEventListener = vi.spyOn(window, 'addEventListener');
    const removeEventListener = vi.spyOn(window, 'removeEventListener');

    const cleanup = installTransportStatus();
    expect(installTransportStatus()).toBe(cleanup);
    expect(addEventListener).toHaveBeenCalledWith('online', expect.any(Function));
    expect(addEventListener).toHaveBeenCalledWith('offline', expect.any(Function));

    cleanup();

    expect(removeEventListener).toHaveBeenCalledWith('online', expect.any(Function));
    expect(removeEventListener).toHaveBeenCalledWith('offline', expect.any(Function));
    addEventListener.mockRestore();
    removeEventListener.mockRestore();
  });

  it('lets a successful wrapped request override an offline browser hint after usable data exists', async () => {
    const cleanup = installTransportStatus();
    const status = useResourceStatusStore(appPinia);
    status.setReady('transport', true);
    window.dispatchEvent(new Event('offline'));
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(null, { status: 200 })));

    await wrappedClient.GET('/api/auth/me');

    expect(status.statusFor('transport')).toBe('ready');
    vi.unstubAllGlobals();
    cleanup();
  });

  it('treats browser connectivity as a hint until a real request fails, then recovers through the wrapped client', async () => {
    const cleanup = installTransportStatus();
    const status = useResourceStatusStore(appPinia);
    status.setReady('transport', true);
    window.dispatchEvent(new Event('offline'));

    expect(status.statusFor('transport')).toBe('ready');

    vi.stubGlobal('fetch', vi.fn().mockResolvedValueOnce(new Response(null, { status: 503 })));
    await wrappedClient.GET('/api/auth/me');
    expect(status.statusFor('transport')).toBe('offline');

    window.dispatchEvent(new Event('online'));
    expect(status.statusFor('transport')).toBe('reconnecting');

    vi.stubGlobal('fetch', vi.fn().mockResolvedValueOnce(new Response(null, { status: 200 })));
    await wrappedClient.GET('/api/auth/me');
    expect(status.statusFor('transport')).toBe('ready');

    vi.unstubAllGlobals();
    cleanup();
  });

  it('removes the wrapped-client outcome handler with the browser listeners', async () => {
    const cleanup = installTransportStatus();
    const status = useResourceStatusStore(appPinia);
    status.setReady('transport', true);
    cleanup();

    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(null, { status: 503 })));
    await wrappedClient.GET('/api/auth/me');

    expect(status.statusFor('transport')).toBe('ready');
    vi.unstubAllGlobals();
  });

  it('keeps usable data non-blocking after a thrown request and recovers on the retry outcome', async () => {
    const cleanup = installTransportStatus();
    const status = useResourceStatusStore(appPinia);
    status.setReady('transport', true);
    vi.stubGlobal('fetch', vi.fn().mockRejectedValueOnce(new TypeError('network unavailable')));

    await expect(wrappedClient.GET('/api/auth/me')).rejects.toThrow('network unavailable');
    expect(status.statusFor('transport')).toBe('error-with-data');
    expect(status.usesFullLoader('transport')).toBe(false);

    vi.stubGlobal('fetch', vi.fn().mockResolvedValueOnce(new Response(null, { status: 200 })));
    await wrappedClient.GET('/api/auth/me');
    expect(status.statusFor('transport')).toBe('ready');

    vi.unstubAllGlobals();
    cleanup();
  });
});

describe('cache invalidation lifecycle', () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it('resolves a loaded workspace slug and purges only the failed resource scope through the real middleware path', async () => {
    const deleteScope = vi.fn().mockResolvedValue(true);
    configureResourceCacheForTest(
      new ResourceCache({
        store: {
          get: vi.fn(),
          putMany: vi.fn().mockResolvedValue(true),
          deleteMany: vi.fn().mockResolvedValue(true),
          deleteScope,
          clear: vi.fn().mockResolvedValue(true),
        },
      }),
    );
    setResourceCachePrincipal('user:019ef171-bbcf-7b90-9be6-5dbb382afd08');
    vi.stubGlobal(
      'fetch',
      vi.fn((request: Request) => {
        if (new URL(request.url).pathname === '/api/workspaces') {
          return Promise.resolve(
            Response.json([
              {
                id: '019ef171-bbcf-7b90-9be6-5dbb382afd08',
                name: 'Atlas',
                slug: 'atlas',
                created_at: '2024-01-01T00:00:00Z',
                updated_at: '2024-01-01T00:00:00Z',
              },
            ]),
          );
        }
        return Promise.resolve(new Response(null, { status: 403 }));
      }),
    );
    const workspace = useWorkspaceStore(appPinia);

    await workspace.loadWorkspaces();
    await wrappedClient.GET('/api/workspaces/{ws}/documents/{slug}', {
      params: { path: { ws: 'atlas', slug: 'document-a' } },
    });

    expect(deleteScope).toHaveBeenCalledWith({
      principal: 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08',
      workspaceId: '019ef171-bbcf-7b90-9be6-5dbb382afd08',
      tagsAny: ['document:document-a'],
    });
  });

  it('blocks cache reuse when an exact alias invalidation fails without globally clearing another workspace', async () => {
    const principal = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const workspaceId = '019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const store = {
      get: vi.fn(),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      deleteScope: vi.fn().mockResolvedValue(false),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store });
    configureResourceCacheForTest(cache);
    setResourceCachePrincipal(principal);
    vi.stubGlobal(
      'fetch',
      vi.fn((request: Request) => {
        if (new URL(request.url).pathname === '/api/workspaces') {
          return Promise.resolve(Response.json([{ id: workspaceId, name: 'Atlas', slug: 'atlas' }]));
        }
        return Promise.resolve(new Response(null, { status: 404 }));
      }),
    );
    const workspace = useWorkspaceStore(appPinia);

    await workspace.loadWorkspaces();
    await wrappedClient.GET('/api/workspaces/{ws}/documents/{slug}', {
      params: { path: { ws: 'atlas', slug: 'document-a' } },
    });

    await expect(
      cache.hydrate({
        key: 'v1|p=user:019ef171-bbcf-7b90-9be6-5dbb382afd08|w=019ef171-bbcf-7b90-9be6-5dbb382afd08|k=note-body|r=document-a|q={}',
        payloadSchema: { safeParse: vi.fn() } as never,
        publish: vi.fn(),
        isCurrent: () => true,
      }),
    ).resolves.toBeNull();
    expect(store.clear).not.toHaveBeenCalled();
  });

  it('blocks cache work before an unknown alias timeout response completes without globally clearing data', async () => {
    vi.useFakeTimers();
    const principal = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const store = {
      get: vi.fn(),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      deleteScope: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store });
    configureResourceCacheForTest(cache);
    setResourceCachePrincipal(principal);
    allowResourceCache();
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(null, { status: 404 })));

    const failedResponse = wrappedClient.GET('/api/workspaces/{ws}/documents/{slug}', {
      params: { path: { ws: 'unknown-timeout', slug: 'document-a' } },
    });
    await vi.advanceTimersByTimeAsync(5 * 60_000);

    await failedResponse;
    const load = vi.fn().mockResolvedValue({ title: 'must-not-run' });
    await cache.revalidate({
      key: 'v1|p=user:019ef171-bbcf-7b90-9be6-5dbb382afd08|w=019ef171-bbcf-7b90-9be6-5dbb382afd08|k=note-body|r=document-a|q={}',
      payloadSchema: undefined as never,
      tags: ['document:document-a'],
      freshForMs: 1,
      retentionForMs: 1,
      load,
      publish: vi.fn(),
      isCurrent: () => true,
    });

    expect(load).not.toHaveBeenCalled();
    expect(store.clear).not.toHaveBeenCalled();
    vi.useRealTimers();
  });

  it('uses the real wrapped-client scopes to preserve unrelated namespaces and blocks unknown aliases until exact purge', async () => {
    const principal = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const workspaceId = '019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const survivingWorkspaceId = '019ef171-bbcf-7b90-9be6-5dbb382afd09';
    const coldEntries = [
      { workspaceId, tags: ['document:document-a'] },
      { workspaceId, tags: ['document:document-b'] },
      { workspaceId: survivingWorkspaceId, tags: ['document:document-a'] },
    ];
    const deleteScope = vi.fn(
      async ({
        workspaceId: scopeWorkspaceId,
        tagsAny,
      }: {
        principal: string;
        workspaceId?: string;
        tagsAny?: readonly string[];
      }) => {
        for (let index = coldEntries.length - 1; index >= 0; index -= 1) {
          const entry = coldEntries[index];
          if (
            entry !== undefined &&
            (scopeWorkspaceId === undefined || entry.workspaceId === scopeWorkspaceId) &&
            (tagsAny === undefined || tagsAny.some((tag) => entry.tags.includes(tag)))
          ) {
            coldEntries.splice(index, 1);
          }
        }
        return true;
      },
    );
    let workspaceListRequests = 0;
    configureResourceCacheForTest(
      new ResourceCache({
        store: {
          get: vi.fn(),
          putMany: vi.fn().mockResolvedValue(true),
          deleteMany: vi.fn().mockResolvedValue(true),
          deleteScope,
          clear: vi.fn().mockResolvedValue(true),
        },
      }),
    );
    setResourceCachePrincipal(principal);
    vi.stubGlobal(
      'fetch',
      vi.fn((request: Request) => {
        const pathname = new URL(request.url).pathname;
        if (pathname === '/api/workspaces') {
          workspaceListRequests += 1;
          return Promise.resolve(
            Response.json([
              { id: workspaceId, name: 'Atlas', slug: 'atlas' },
              ...(workspaceListRequests === 1
                ? []
                : [{ id: survivingWorkspaceId, name: 'Unknown', slug: 'unknown' }]),
            ]),
          );
        }
        if (pathname === '/api/workspaces/atlas/documents/document-a') {
          return Promise.resolve(new Response(null, { status: 404 }));
        }
        if (pathname === '/api/workspaces/atlas') {
          return Promise.resolve(new Response(null, { status: 403 }));
        }
        return Promise.resolve(new Response(null, { status: 403 }));
      }),
    );
    const workspace = useWorkspaceStore(appPinia);

    await workspace.loadWorkspaces();
    await wrappedClient.GET('/api/workspaces/{ws}/documents/{slug}', {
      params: { path: { ws: 'atlas', slug: 'document-a' } },
    });
    expect(coldEntries).toEqual([
      { workspaceId, tags: ['document:document-b'] },
      { workspaceId: survivingWorkspaceId, tags: ['document:document-a'] },
    ]);

    await wrappedClient.GET('/api/workspaces/{ws}', { params: { path: { ws: 'atlas' } } });
    expect(coldEntries).toEqual([{ workspaceId: survivingWorkspaceId, tags: ['document:document-a'] }]);

    let unknownCompleted = false;
    const unknownRequest = wrappedClient
      .GET('/api/workspaces/{ws}/documents/{slug}', {
        params: { path: { ws: 'unknown', slug: 'document-a' } },
      })
      .then(() => {
        unknownCompleted = true;
      });
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(deleteScope).toHaveBeenCalledTimes(2);
    expect(coldEntries).toEqual([{ workspaceId: survivingWorkspaceId, tags: ['document:document-a'] }]);
    expect(unknownCompleted).toBe(false);

    await workspace.loadWorkspaces();
    await unknownRequest;

    expect(deleteScope).toHaveBeenCalledTimes(3);
    expect(coldEntries).toEqual([]);
  });
});
