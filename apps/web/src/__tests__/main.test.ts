import { flushPromises } from '@vue/test-utils';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ComponentPublicInstance } from 'vue';

const { disposeWorkspaceLiveUpdates } = vi.hoisted(() => ({
  disposeWorkspaceLiveUpdates: vi.fn(),
}));

const { tauriInvoke, tauriListen } = vi.hoisted(() => ({
  tauriInvoke: vi.fn(),
  tauriListen: vi.fn(),
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
vi.mock('@tauri-apps/api/core', () => ({ invoke: tauriInvoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: tauriListen }));

import { wrappedClient } from '@/api/wrapper';
import {
  allowResourceCache,
  configureResourceCacheForTest,
  setResourceCachePrincipal,
} from '@/cache/cacheRuntime';
import { ResourceCache } from '@/cache/resourceCache';
import {
  appPinia,
  bootstrapPlatformTransport,
  installTransportStatus,
  mountAfterAuthenticationInitialization,
  registerWorkspaceLiveUpdatesPagehide,
} from '@/main';
import { createBrowserPlatformTransport } from '@/platform/browser';
import { createDesktopPlatformTransport, type DesktopBridge } from '@/platform/desktop';
import { useResourceStatusStore } from '@/stores/resourceStatus';
import { useWorkspaceStore } from '@/stores/workspace';

describe('workspace live update page lifecycle', () => {
  afterEach(() => {
    disposeWorkspaceLiveUpdates.mockClear();
    tauriInvoke.mockReset();
    tauriListen.mockReset();
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

describe('platform transport bootstrap', () => {
  it('mounts only after desktop resume has resolved the authenticated identity', async () => {
    let resolveResume: (() => void) | undefined;
    const resume = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          resolveResume = resolve;
        }),
    );
    const mount = vi.fn();

    const initialization = mountAfterAuthenticationInitialization(resume, mount);

    expect(resume).toHaveBeenCalledOnce();
    expect(mount).not.toHaveBeenCalled();

    resolveResume?.();
    await initialization;

    expect(mount).toHaveBeenCalledOnce();
  });

  it('mounts only after credential-loss initialization resolves to an unauthenticated state', async () => {
    let resolveInitialization: (() => void) | undefined;
    let authenticated = true;
    const initialize = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          resolveInitialization = () => {
            authenticated = false;
            resolve();
          };
        }),
    );
    const mount = vi.fn<() => ComponentPublicInstance>(() => {
      expect(authenticated).toBe(false);
      return {} as ComponentPublicInstance;
    });

    const initialization = mountAfterAuthenticationInitialization(initialize, mount);

    expect(mount).not.toHaveBeenCalled();
    resolveInitialization?.();
    await initialization;

    expect(mount).toHaveBeenCalledOnce();
  });

  it('mounts the unauthenticated application after rejected initialization', async () => {
    const mount = vi.fn();

    await mountAfterAuthenticationInitialization(
      () => Promise.reject(new Error('credential storage unavailable')),
      mount,
    );

    expect(mount).toHaveBeenCalledOnce();
  });

  it('selects browser transport without changing wrapped-client cookie, CSRF, or EventSource behavior', async () => {
    const transport = createBrowserPlatformTransport();
    const fetch = vi.fn().mockResolvedValue(new Response(null, { status: 200 }));
    const EventSource = vi.fn();
    vi.stubGlobal('fetch', fetch);
    vi.stubGlobal('EventSource', EventSource);

    await transport.login({ username: 'alice', password: 'pass' });
    await transport.me();
    await transport.resume();
    await transport.logout();
    transport.createWorkspaceEventSource('acme');

    const requests = fetch.mock.calls.map(([request]) => request as Request);
    expect(requests.map((request) => new URL(request.url).pathname)).toEqual([
      '/api/auth/login',
      '/api/auth/me',
      '/api/auth/me',
      '/api/auth/logout',
    ]);
    for (const request of requests) {
      expect(request.credentials).toBe('include');
    }
    expect(requests[0]?.headers.get('X-Atlas-CSRF')).toBe('1');
    expect(EventSource).toHaveBeenCalledExactlyOnceWith('/api/workspaces/acme/events');
    vi.unstubAllGlobals();
  });

  it('uses the official Tauri API modules when the global bridge is disabled', async () => {
    tauriInvoke.mockImplementation(async (command: string) => {
      if (command === 'desktop_workspace_events_subscribe') {
        return { data: { generation: 1 }, error: undefined };
      }
      return { data: {}, error: undefined };
    });
    tauriListen.mockResolvedValue(() => {});
    vi.stubGlobal('__TAURI__', undefined);

    const transport = createDesktopPlatformTransport();
    const source = transport.createWorkspaceEventSource('acme');

    await transport.login({ username: 'alice', password: 'pass' });
    await transport.me();
    await transport.resume();
    await transport.logout();
    await flushPromises();

    expect(tauriListen).toHaveBeenCalledWith('atlas://session-action', expect.any(Function));
    expect(tauriListen).toHaveBeenCalledWith('atlas://workspace-event', expect.any(Function));
    expect(tauriListen).toHaveBeenCalledWith('atlas://workspace-closed', expect.any(Function));
    expect(tauriListen).toHaveBeenCalledWith('atlas://workspace-resync', expect.any(Function));
    expect(tauriInvoke).toHaveBeenCalledWith('desktop_workspace_events_subscribe', {
      workspaceSlug: 'acme',
    });
    expect(tauriInvoke).toHaveBeenCalledWith('desktop_auth_login', {
      credentials: {
        username: 'alice',
        password: 'pass',
      },
    });
    expect(tauriInvoke).toHaveBeenCalledWith('desktop_auth_me');
    expect(tauriInvoke).toHaveBeenCalledWith('desktop_auth_resume');
    expect(tauriInvoke).toHaveBeenCalledWith('desktop_auth_logout');
    expect(source.readyState).toBe(1);
    vi.unstubAllGlobals();
  });

  it('preserves desktop command failures from the official bridge', async () => {
    tauriInvoke.mockRejectedValue(new Error('desktop unavailable'));
    tauriListen.mockResolvedValue(() => {});

    const transport = createDesktopPlatformTransport();

    await expect(transport.login({ username: 'alice', password: 'pass' })).rejects.toThrow(
      'desktop unavailable',
    );
  });

  it('dispatches desktop commands and normalized realtime events without exposing a token', async () => {
    const invoke = vi.fn(async (command: string) => {
      if (command === 'desktop_workspace_events_subscribe') {
        return { data: { generation: 1 }, error: undefined };
      }
      return { data: { username: 'alice' }, error: undefined };
    });
    let receive: ((event: { payload: unknown }) => void) | undefined;
    const listen: DesktopBridge['listen'] = async (eventName, handler) => {
      if (eventName === 'atlas://workspace-event') {
        receive = handler as (event: { payload: unknown }) => void;
      }
      return () => {};
    };
    const transport = createDesktopPlatformTransport({ invoke, listen });
    const source = transport.createWorkspaceEventSource('acme');
    const onTaskCreated = vi.fn();
    source.addEventListener('task.created', onTaskCreated);

    await transport.me();
    await flushPromises();
    receive?.({ payload: { event_type: 'task.created', data: { task_id: 'task-1' } } });
    await Promise.resolve();

    expect(invoke).toHaveBeenCalledWith('desktop_auth_me');
    expect(onTaskCreated).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({
        data: JSON.stringify({ event_type: 'task.created', data: { task_id: 'task-1' } }),
      }),
    );
    expect(JSON.stringify(invoke.mock.calls)).not.toContain('token');
  });

  it('cancels the Rust workspace transport when a desktop source closes', async () => {
    const invoke = vi.fn(async (command: string) => {
      if (command === 'desktop_workspace_events_subscribe') {
        return { data: { generation: 4 }, error: undefined };
      }
      return {};
    });
    const transport = createDesktopPlatformTransport({
      invoke,
      listen: async () => () => {},
    });
    const source = transport.createWorkspaceEventSource('acme');

    await flushPromises();
    source.close();
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith('desktop_workspace_events_stop', {
      workspaceSlug: 'acme',
      generation: 4,
    });
  });

  it('chooses the adapter once at bootstrap rather than in components', () => {
    const transport = bootstrapPlatformTransport({
      isDesktop: () => true,
      browser: () => ({ kind: 'browser' }),
      desktop: () => ({ kind: 'desktop' }),
    });

    expect(transport).toEqual({ kind: 'desktop' });
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
