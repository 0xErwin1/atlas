import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const { invalidateLiveResourceCache, resourceCacheEpoch } = vi.hoisted(() => ({
  invalidateLiveResourceCache: vi.fn().mockResolvedValue(true),
  resourceCacheEpoch: { value: 0 },
}));

vi.mock('@/cache/cacheRuntime', () => ({ invalidateLiveResourceCache, resourceCacheEpoch }));

import { wrappedClient } from '@/api/wrapper';
import { createWorkspaceLiveUpdatesBroker, type WorkspaceLiveUpdate } from '@/lib/workspaceLiveUpdates';
import {
  createDesktopGateLiveUpdateObserver,
  createDesktopPlatformTransport,
  type DesktopBridge,
  getDesktopGateLiveUpdateObserver,
} from '@/platform/desktop';
import {
  type PlatformTransport,
  resetPlatformTransportForTest,
  setPlatformTransport,
} from '@/platform/transport';

type WorkspaceProbeResponse = Awaited<ReturnType<typeof wrappedClient.GET>>;

function workspaceProbeResponse(status: number): WorkspaceProbeResponse {
  return {
    data: undefined,
    error: undefined as never,
    response: new Response(null, { status }),
  };
}

class FakeEventSource {
  static instances: FakeEventSource[] = [];

  readonly url: string;
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSED = 2;

  closed = false;
  readyState = FakeEventSource.CONNECTING;
  onopen: (() => void) | null = null;
  onerror: (() => void) | null = null;
  onmessage: ((event: MessageEvent) => void) | null = null;
  private listeners: Record<string, ((event: Event) => void)[]> = {};

  constructor(url: string) {
    this.url = url;
    FakeEventSource.instances.push(this);
  }

  addEventListener(type: string, listener: (event: Event) => void): void {
    this.listeners[type] = [...(this.listeners[type] ?? []), listener];
  }

  close(): void {
    this.closed = true;
    this.readyState = FakeEventSource.CLOSED;
  }

  emitOpen(): void {
    this.readyState = FakeEventSource.OPEN;
    this.onopen?.();
  }

  emitError(readyState: number): void {
    this.readyState = readyState;
    this.onerror?.();
  }

  listenerCount(type: string): number {
    return this.listeners[type]?.length ?? 0;
  }

  emit(type: string, data: string): void {
    const event = new MessageEvent(type, { data });
    if (type === 'message') this.onmessage?.(event);
    for (const listener of this.listeners[type] ?? []) listener(event);
  }
}

function event(type: string, data: unknown): string {
  return JSON.stringify({
    id: 'event-1',
    event_type: type,
    version: 1,
    source: 'test',
    workspace_id: 'workspace-1',
    occurred_at: '2026-01-01T00:00:00Z',
    actor: { type: 'user', id: 'user-1' },
    data,
  });
}

function handlers(
  overrides: Partial<{
    onEvent: (update: WorkspaceLiveUpdate) => void;
    onResync: () => void;
    onReconnectFailed: () => void;
  }> = {},
) {
  return {
    onEvent: vi.fn(),
    onResync: vi.fn(),
    onReconnectFailed: vi.fn(),
    ...overrides,
  };
}

function platformTransport(
  createWorkspaceEventSource: PlatformTransport['createWorkspaceEventSource'],
): PlatformTransport {
  return {
    isDesktop: true,
    login: async () => ({}),
    me: async () => ({}),
    resume: async () => ({}),
    logout: async () => ({}),
    getOrigin: async () => ({ data: { origin: 'https://atlas.test' } }),
    setOrigin: async (origin) => ({ data: { origin } }),
    createWorkspaceEventSource,
  };
}

describe('workspace live updates broker', () => {
  beforeEach(() => {
    FakeEventSource.instances = [];
    invalidateLiveResourceCache.mockClear();
    resourceCacheEpoch.value = 0;
    resetPlatformTransportForTest();
    vi.useFakeTimers();
    vi.stubGlobal('EventSource', FakeEventSource);
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('shares one source across concurrent subscribers and fans events out exactly once', () => {
    const broker = createWorkspaceLiveUpdatesBroker();
    const first = handlers();
    const second = handlers();

    const firstSubscription = broker.acquire('acme', first);
    const secondSubscription = broker.acquire('acme', second);

    expect(FakeEventSource.instances).toHaveLength(1);
    expect(FakeEventSource.instances[0]?.url).toBe('/api/workspaces/acme/events');

    FakeEventSource.instances[0]?.emit('task.created', event('task.created', { task_id: 'task-1' }));

    expect(first.onEvent).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ type: 'task.created', data: { task_id: 'task-1' } }),
    );
    expect(second.onEvent).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ type: 'task.created', data: { task_id: 'task-1' } }),
    );

    firstSubscription.release();
    secondSubscription.release();
    vi.advanceTimersByTime(30_000);
  });

  it('opens realtime through the selected platform source factory', () => {
    const createWorkspaceEventSource = vi.fn(
      (workspaceSlug: string) => new FakeEventSource(`desktop://${workspaceSlug}`),
    );
    setPlatformTransport(platformTransport(createWorkspaceEventSource));
    const broker = createWorkspaceLiveUpdatesBroker();
    const subscription = broker.acquire('acme', handlers());

    expect(createWorkspaceEventSource).toHaveBeenCalledExactlyOnceWith('acme');
    expect(FakeEventSource.instances[0]?.url).toBe('desktop://acme');

    subscription.release();
    vi.advanceTimersByTime(30_000);
  });

  it('uses the browser EventSource when acquired before main selects a transport', () => {
    const broker = createWorkspaceLiveUpdatesBroker();
    const subscription = broker.acquire('acme', handlers());

    expect(FakeEventSource.instances).toHaveLength(1);
    expect(FakeEventSource.instances[0]?.url).toBe('/api/workspaces/acme/events');

    subscription.release();
    vi.advanceTimersByTime(30_000);
  });

  it('dispatches one normalized desktop event through the existing broker exactly once', async () => {
    let receive: ((event: { payload: unknown }) => void) | undefined;
    const bridge: DesktopBridge = {
      invoke: vi.fn().mockResolvedValue(undefined),
      listen: async (eventName, handler) => {
        if (eventName === 'atlas://workspace-event') {
          receive = handler as (event: { payload: unknown }) => void;
        }
        return () => {};
      },
    };
    setPlatformTransport(createDesktopPlatformTransport(bridge));
    const broker = createWorkspaceLiveUpdatesBroker();
    const subscriber = handlers();
    const subscription = broker.acquire('acme', subscriber);

    await Promise.resolve();
    const envelope = JSON.parse(event('task.created', { task_id: 'task-1' }));
    receive?.({ payload: envelope });

    expect(invalidateLiveResourceCache).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ event_type: 'task.created' }),
      'acme',
    );
    expect(subscriber.onEvent).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ type: 'task.created', data: { task_id: 'task-1' } }),
    );
    subscription.release();
    vi.advanceTimersByTime(30_000);
  });

  it('observes one non-sensitive desktop event after the broker consumes it', async () => {
    let receive: ((event: { payload: unknown }) => void) | undefined;
    const bridge: DesktopBridge = {
      invoke: vi.fn().mockResolvedValue(undefined),
      listen: async (eventName, handler) => {
        if (eventName === 'atlas://workspace-event') {
          receive = handler as (event: { payload: unknown }) => void;
        }
        return () => {};
      },
    };
    const observer = createDesktopGateLiveUpdateObserver(true);
    const observed = vi.fn();
    observer.subscribe(observed);
    setPlatformTransport(createDesktopPlatformTransport(bridge));
    const broker = createWorkspaceLiveUpdatesBroker({ desktopGateObserver: observer });
    const subscription = broker.acquire('acme', handlers());

    await Promise.resolve();
    receive?.({ payload: JSON.parse(event('task.created', { task_id: 'task-1', token: 'secret' })) });

    expect(observer.snapshot()).toEqual({
      count: 1,
      eventType: 'task.created',
      status: 'event',
      workspaceSlug: 'acme',
    });
    expect(observed).toHaveBeenCalledExactlyOnceWith({
      count: 1,
      eventType: 'task.created',
      status: 'event',
      workspaceSlug: 'acme',
    });
    subscription.release();
    vi.advanceTimersByTime(30_000);
  });

  it('observes reconnect and resync status without incrementing the event count', async () => {
    let receiveClosed: ((event: { payload: unknown }) => void) | undefined;
    const bridge: DesktopBridge = {
      invoke: vi.fn().mockResolvedValue(undefined),
      listen: async (eventName, handler) => {
        if (eventName === 'atlas://workspace-closed') {
          receiveClosed = handler as (event: { payload: unknown }) => void;
        }
        return () => {};
      },
    };
    const observer = createDesktopGateLiveUpdateObserver(true);
    const observed = vi.fn();
    observer.subscribe(observed);
    setPlatformTransport(createDesktopPlatformTransport(bridge));
    vi.spyOn(Math, 'random').mockReturnValue(0);
    const broker = createWorkspaceLiveUpdatesBroker({ desktopGateObserver: observer });
    const subscription = broker.acquire('acme', handlers());

    await Promise.resolve();
    receiveClosed?.({ payload: { workspace_slug: 'acme' } });
    vi.advanceTimersByTime(500);
    await Promise.resolve();

    expect(observer.snapshot()).toEqual({ count: 0, status: 'resync' });
    expect(observed).toHaveBeenNthCalledWith(1, { count: 0, status: 'reconnecting' });
    expect(observed).toHaveBeenNthCalledWith(2, { count: 0, status: 'reconnected' });
    expect(observed).toHaveBeenNthCalledWith(3, { count: 0, status: 'resync' });
    subscription.release();
    vi.advanceTimersByTime(30_000);
  });

  it('does not expose a gate observer when the desktop gate build flag is absent', () => {
    expect(getDesktopGateLiveUpdateObserver()).toBeNull();
  });

  it('reconnects when the Rust desktop transport reports a late stream closure', async () => {
    let receiveClosed: ((event: { payload: unknown }) => void) | undefined;
    const bridge: DesktopBridge = {
      invoke: vi.fn().mockResolvedValue(undefined),
      listen: async (eventName, handler) => {
        if (eventName === 'atlas://workspace-closed') {
          receiveClosed = handler as (event: { payload: unknown }) => void;
        }
        return () => {};
      },
    };
    setPlatformTransport(createDesktopPlatformTransport(bridge));
    vi.spyOn(Math, 'random').mockReturnValue(0);
    const broker = createWorkspaceLiveUpdatesBroker();
    const subscription = broker.acquire('acme', handlers());

    await Promise.resolve();
    receiveClosed?.({ payload: { workspace_slug: 'acme' } });
    vi.advanceTimersByTime(30_000);

    expect(bridge.invoke).toHaveBeenCalledWith('desktop_workspace_events_subscribe', {
      workspaceSlug: 'acme',
    });
    subscription.release();
    vi.advanceTimersByTime(30_000);
  });

  it('delivers a Rust server-resync signal once through the desktop source', async () => {
    let receiveResync: ((event: { payload: unknown }) => void) | undefined;
    const bridge: DesktopBridge = {
      invoke: vi.fn().mockResolvedValue(undefined),
      listen: async (eventName, handler) => {
        if (eventName === 'atlas://workspace-resync') {
          receiveResync = handler as (event: { payload: unknown }) => void;
        }
        return () => {};
      },
    };
    const source = createDesktopPlatformTransport(bridge).createWorkspaceEventSource('acme');
    const onResync = vi.fn();
    source.addEventListener('resync', onResync);

    await Promise.resolve();
    receiveResync?.({ payload: { workspace_slug: 'acme' } });

    expect(onResync).toHaveBeenCalledExactlyOnceWith(expect.any(Event));
    source.close();
  });

  it('starts cache invalidation before dispatching a valid task event to subscribers', () => {
    const broker = createWorkspaceLiveUpdatesBroker();
    const order: string[] = [];
    invalidateLiveResourceCache.mockImplementation(() => {
      order.push('cache');
      return Promise.resolve(true);
    });
    const onEvent = vi.fn(() => order.push('subscriber'));
    const subscription = broker.acquire('acme', { ...handlers(), onEvent });

    FakeEventSource.instances[0]?.emit('task.created', event('task.created', { task_id: 'task-1' }));

    expect(onEvent).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ type: 'task.created' }));
    expect(invalidateLiveResourceCache).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ event_type: 'task.created' }),
      'acme',
    );
    expect(order).toEqual(['cache', 'subscriber']);
    subscription.release();
    vi.advanceTimersByTime(30_000);
  });

  it('fans resync and reconnect failures out exactly once to current subscribers', () => {
    const broker = createWorkspaceLiveUpdatesBroker();
    const first = handlers();
    const second = handlers();

    const firstSubscription = broker.acquire('acme', first);
    const secondSubscription = broker.acquire('acme', second);

    FakeEventSource.instances[0]?.emit('resync', 'reload');
    broker.notifyReconnectFailed();

    expect(first.onResync).toHaveBeenCalledTimes(1);
    expect(second.onResync).toHaveBeenCalledTimes(1);
    expect(first.onReconnectFailed).toHaveBeenCalledTimes(1);
    expect(second.onReconnectFailed).toHaveBeenCalledTimes(1);

    firstSubscription.release();
    secondSubscription.release();
    vi.advanceTimersByTime(30_000);
  });

  it('stales the current broker workspace before dispatching a resync', () => {
    const broker = createWorkspaceLiveUpdatesBroker();
    const order: string[] = [];
    invalidateLiveResourceCache.mockImplementation(() => {
      order.push('cache');
      return Promise.resolve(true);
    });
    const onResync = vi.fn(() => order.push('subscriber'));
    const subscription = broker.acquire('acme', { ...handlers(), onResync });

    FakeEventSource.instances[0]?.emit('resync', 'reload');

    expect(onResync).toHaveBeenCalledExactlyOnceWith();
    expect(invalidateLiveResourceCache).toHaveBeenCalledExactlyOnceWith(undefined, 'acme');
    expect(order).toEqual(['cache', 'subscriber']);
    subscription.release();
    vi.advanceTimersByTime(30_000);
  });

  it('stales the current broker workspace and resyncs subscribers for malformed envelopes', () => {
    const broker = createWorkspaceLiveUpdatesBroker();
    const subscriber = handlers();
    const consoleDebug = vi.spyOn(console, 'debug').mockImplementation(() => {});
    const subscription = broker.acquire('acme', subscriber);

    FakeEventSource.instances[0]?.emit('task.created', '{not-json');

    expect(invalidateLiveResourceCache).toHaveBeenCalledExactlyOnceWith(undefined, 'acme');
    expect(subscriber.onEvent).not.toHaveBeenCalled();
    expect(subscriber.onResync).toHaveBeenCalledExactlyOnceWith();
    expect(consoleDebug).toHaveBeenCalledOnce();
    subscription.release();
    vi.advanceTimersByTime(30_000);
  });

  it('generation-fails late principal-A callbacks before they can invalidate principal-B cache state', () => {
    resourceCacheEpoch.value = 1;
    const consoleDebug = vi.spyOn(console, 'debug').mockImplementation(() => {});
    const broker = createWorkspaceLiveUpdatesBroker();
    const principalA = broker.acquire('acme', handlers());
    const sourceA = FakeEventSource.instances[0];

    resourceCacheEpoch.value = 2;
    sourceA?.emit('task.created', '{not-json');

    expect(invalidateLiveResourceCache).not.toHaveBeenCalled();

    const principalB = broker.acquire('acme', handlers());
    FakeEventSource.instances[1]?.emit('task.created', '{not-json');

    expect(invalidateLiveResourceCache).toHaveBeenCalledExactlyOnceWith(undefined, 'acme');
    principalA.release();
    principalB.release();
    vi.advanceTimersByTime(30_000);
    consoleDebug.mockRestore();
  });

  it('passes the current broker alias with valid envelopes while preserving one source', () => {
    const broker = createWorkspaceLiveUpdatesBroker();
    const first = broker.acquire('acme', handlers());
    const second = broker.acquire('acme', handlers());

    FakeEventSource.instances[0]?.emit('task.created', event('task.created', { task_id: 'task-1' }));

    expect(FakeEventSource.instances).toHaveLength(1);
    expect(invalidateLiveResourceCache).toHaveBeenCalledWith(expect.any(Object), 'acme');
    first.release();
    second.release();
    vi.advanceTimersByTime(30_000);
  });

  it('excludes released subscribers from event, resync, and reconnect-failed fan-out', () => {
    const broker = createWorkspaceLiveUpdatesBroker();
    const released = handlers();
    const current = handlers();

    const releasedSubscription = broker.acquire('acme', released);
    const currentSubscription = broker.acquire('acme', current);
    releasedSubscription.release();

    FakeEventSource.instances[0]?.emit('task.created', event('task.created', { task_id: 'task-1' }));
    FakeEventSource.instances[0]?.emit('resync', 'reload');
    broker.notifyReconnectFailed();

    expect(released.onEvent).not.toHaveBeenCalled();
    expect(released.onResync).not.toHaveBeenCalled();
    expect(released.onReconnectFailed).not.toHaveBeenCalled();
    expect(current.onEvent).toHaveBeenCalledTimes(1);
    expect(current.onResync).toHaveBeenCalledTimes(1);
    expect(current.onReconnectFailed).toHaveBeenCalledTimes(1);

    currentSubscription.release();
    vi.advanceTimersByTime(30_000);
  });

  it('prevents self-release and releasing another subscriber from affecting current dispatch', () => {
    const broker = createWorkspaceLiveUpdatesBroker();
    const self = handlers();
    const other = handlers();
    const releaser = handlers();
    let selfSubscription: ReturnType<typeof broker.acquire>;
    let otherSubscription: ReturnType<typeof broker.acquire>;

    selfSubscription = broker.acquire('acme', {
      ...self,
      onEvent: (update) => {
        self.onEvent(update);
        selfSubscription.release();
      },
    });
    const releaserSubscription = broker.acquire('acme', {
      ...releaser,
      onEvent: () => otherSubscription.release(),
    });
    otherSubscription = broker.acquire('acme', other);

    FakeEventSource.instances[0]?.emit('task.created', event('task.created', { task_id: 'task-1' }));

    expect(self.onEvent).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ type: 'task.created', data: { task_id: 'task-1' } }),
    );
    expect(other.onEvent).not.toHaveBeenCalled();

    FakeEventSource.instances[0]?.emit('task.created', event('task.created', { task_id: 'task-2' }));

    expect(self.onEvent).toHaveBeenCalledTimes(1);
    expect(other.onEvent).not.toHaveBeenCalled();

    releaserSubscription.release();
    vi.advanceTimersByTime(30_000);
  });

  it('isolates callback failures and defers subscribers acquired during dispatch until the next notification', () => {
    const broker = createWorkspaceLiveUpdatesBroker();
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});
    const throwing = handlers({
      onEvent: () => {
        throw new Error('subscriber failure');
      },
    });
    const acquiredDuringDispatch = handlers();
    let acquired = false;
    const acquiredSubscription: { current: ReturnType<typeof broker.acquire> | null } = { current: null };

    const throwingSubscription = broker.acquire('acme', throwing);
    const acquirerSubscription = broker.acquire('acme', {
      onEvent: () => {
        if (!acquired) {
          acquiredSubscription.current = broker.acquire('acme', acquiredDuringDispatch);
          acquired = true;
        }
      },
      onResync: vi.fn(),
      onReconnectFailed: vi.fn(),
    });

    FakeEventSource.instances[0]?.emit('task.created', event('task.created', { task_id: 'task-1' }));

    expect(acquired).toBe(true);
    expect(consoleError).toHaveBeenCalledTimes(1);
    expect(acquiredDuringDispatch.onEvent).not.toHaveBeenCalled();

    FakeEventSource.instances[0]?.emit('task.created', event('task.created', { task_id: 'task-2' }));

    expect(acquiredDuringDispatch.onEvent).toHaveBeenCalledTimes(1);

    throwingSubscription.release();
    acquirerSubscription.release();
    acquiredSubscription.current?.release();
    vi.advanceTimersByTime(30_000);
  });

  it('keeps the source during the idle grace period, cancels teardown on reacquisition, and closes after 30 seconds without subscribers', () => {
    const broker = createWorkspaceLiveUpdatesBroker();
    const firstSubscription = broker.acquire('acme', handlers());
    const source = FakeEventSource.instances[0];

    firstSubscription.release();
    vi.advanceTimersByTime(29_999);
    expect(source?.closed).toBe(false);

    const secondSubscription = broker.acquire('acme', handlers());
    vi.advanceTimersByTime(1);
    expect(source?.closed).toBe(false);

    secondSubscription.release();
    vi.advanceTimersByTime(30_000);
    expect(source?.closed).toBe(true);
  });

  async function exhaustRecovery(): Promise<void> {
    vi.spyOn(Math, 'random').mockReturnValue(0);
    const source = FakeEventSource.instances[0];

    for (let attempt = 0; attempt < 11; attempt += 1) {
      FakeEventSource.instances.at(-1)?.emitError(FakeEventSource.CLOSED);
      vi.advanceTimersByTime(30_000);
    }

    await Promise.resolve();
    expect(source).toBeDefined();
  }

  describe('authorization probe', () => {
    it('invalidates the registered session handler for a current 401 probe response', async () => {
      vi.spyOn(wrappedClient, 'GET').mockResolvedValue(workspaceProbeResponse(401));
      const broker = createWorkspaceLiveUpdatesBroker();
      const invalidateSession = vi.fn();
      broker.setAuthorizationInvalidator(invalidateSession);
      const subscription = broker.acquire('acme', handlers());

      await exhaustRecovery();

      expect(invalidateSession).toHaveBeenCalledExactlyOnceWith();
      expect(FakeEventSource.instances[0]?.closed).toBe(true);
      subscription.release();
    });

    it.each([
      403, 404,
    ])('terminates only the current workspace lifetime for a %i probe response', async (status) => {
      vi.spyOn(wrappedClient, 'GET').mockResolvedValue(workspaceProbeResponse(status));
      const broker = createWorkspaceLiveUpdatesBroker();
      const invalidateSession = vi.fn();
      broker.setAuthorizationInvalidator(invalidateSession);
      const subscription = broker.acquire('acme', handlers());

      await exhaustRecovery();

      expect(FakeEventSource.instances[0]?.closed).toBe(true);
      expect(invalidateSession).not.toHaveBeenCalled();
      subscription.release();
    });

    it('keeps the workspace alive for other probe statuses and transport rejection', async () => {
      const get = vi.spyOn(wrappedClient, 'GET');
      get.mockResolvedValueOnce(workspaceProbeResponse(500));
      const broker = createWorkspaceLiveUpdatesBroker();
      const invalidateSession = vi.fn();
      broker.setAuthorizationInvalidator(invalidateSession);
      const subscription = broker.acquire('acme', handlers());

      await exhaustRecovery();

      const sourceCount = FakeEventSource.instances.length;
      const secondSubscription = broker.acquire('acme', handlers());
      expect(FakeEventSource.instances).toHaveLength(sourceCount);
      expect(invalidateSession).not.toHaveBeenCalled();

      get.mockRejectedValueOnce(new Error('network unavailable'));
      FakeEventSource.instances[0]?.emitError(FakeEventSource.CLOSED);
      await Promise.resolve();

      expect(FakeEventSource.instances).toHaveLength(sourceCount);
      expect(invalidateSession).not.toHaveBeenCalled();
      secondSubscription.release();
      subscription.release();
      vi.advanceTimersByTime(30_000);
    });

    it('ignores stale probe fulfillment and rejection after source recovery or workspace replacement', async () => {
      let resolveProbe: ((value: WorkspaceProbeResponse) => void) | undefined;
      let rejectProbe: ((reason: Error) => void) | undefined;
      const get = vi.spyOn(wrappedClient, 'GET');
      get.mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveProbe = resolve;
          }),
      );
      get.mockImplementationOnce(
        () =>
          new Promise((_, reject) => {
            rejectProbe = reject;
          }),
      );
      const broker = createWorkspaceLiveUpdatesBroker();
      const invalidateSession = vi.fn();
      broker.setAuthorizationInvalidator(invalidateSession);
      const acme = broker.acquire('acme', handlers());

      await exhaustRecovery();
      const sourceCount = FakeEventSource.instances.length;
      window.dispatchEvent(new Event('focus'));
      vi.advanceTimersByTime(300);
      expect(FakeEventSource.instances).toHaveLength(sourceCount + 1);

      resolveProbe?.(workspaceProbeResponse(401));
      await Promise.resolve();
      expect(invalidateSession).not.toHaveBeenCalled();
      expect(FakeEventSource.instances.at(-1)?.closed).toBe(false);

      FakeEventSource.instances.at(-1)?.emitError(FakeEventSource.CLOSED);
      await Promise.resolve();
      const globex = broker.acquire('globex', handlers());
      rejectProbe?.(new Error('stale network failure'));
      await Promise.resolve();

      expect(invalidateSession).not.toHaveBeenCalled();
      expect(FakeEventSource.instances.at(-1)?.url).toBe('/api/workspaces/globex/events');
      expect(FakeEventSource.instances.at(-1)?.closed).toBe(false);
      acme.release();
      globex.release();
      vi.advanceTimersByTime(30_000);
    });
  });

  describe('centralized ATL-113 recovery', () => {
    it('reopens only after a CLOSED error with bounded backoff and stops after the retry cap', () => {
      vi.spyOn(Math, 'random').mockReturnValue(0);
      const broker = createWorkspaceLiveUpdatesBroker();
      const reconnectFailed = vi.fn();

      const subscription = broker.acquire('acme', { ...handlers(), onReconnectFailed: reconnectFailed });

      for (let attempt = 0; attempt < 10; attempt += 1) {
        const current = FakeEventSource.instances.at(-1);
        current?.emitError(FakeEventSource.CLOSED);
        vi.advanceTimersByTime(30_000);
      }

      expect(FakeEventSource.instances).toHaveLength(11);

      FakeEventSource.instances.at(-1)?.emitError(FakeEventSource.CLOSED);
      vi.advanceTimersByTime(30_000);

      expect(FakeEventSource.instances).toHaveLength(11);
      expect(reconnectFailed).toHaveBeenCalledExactlyOnceWith();
      expect(invalidateLiveResourceCache).toHaveBeenCalledExactlyOnceWith(undefined, 'acme');
      subscription.release();
      vi.advanceTimersByTime(30_000);
    });

    it('does not overlap the native CONNECTING retry with a broker reconnect', () => {
      const broker = createWorkspaceLiveUpdatesBroker();

      const subscription = broker.acquire('acme', handlers());
      FakeEventSource.instances[0]?.emitError(FakeEventSource.CONNECTING);
      vi.advanceTimersByTime(60_000);

      expect(FakeEventSource.instances).toHaveLength(1);
      subscription.release();
      vi.advanceTimersByTime(30_000);
    });

    it('replaces a pre-existing CONNECTING source once on foreground without replacing that explicit attempt', () => {
      const broker = createWorkspaceLiveUpdatesBroker();
      const subscription = broker.acquire('acme', handlers());
      const nativeSource = FakeEventSource.instances[0];

      window.dispatchEvent(new Event('focus'));
      vi.advanceTimersByTime(300);

      expect(nativeSource?.closed).toBe(true);
      expect(FakeEventSource.instances).toHaveLength(2);
      const foregroundSource = FakeEventSource.instances[1];

      window.dispatchEvent(new Event('focus'));
      vi.advanceTimersByTime(300);

      expect(foregroundSource?.closed).toBe(false);
      expect(FakeEventSource.instances).toHaveLength(2);

      foregroundSource?.emitError(FakeEventSource.CONNECTING);
      vi.advanceTimersByTime(60_000);
      expect(FakeEventSource.instances).toHaveLength(2);

      subscription.release();
      vi.advanceTimersByTime(30_000);
    });

    it('suppresses first-open resync and resyncs all subscribers after a reconnect opens', () => {
      vi.spyOn(Math, 'random').mockReturnValue(0);
      const broker = createWorkspaceLiveUpdatesBroker();
      const first = handlers();
      const second = handlers();

      const firstSubscription = broker.acquire('acme', first);
      const secondSubscription = broker.acquire('acme', second);

      const source = FakeEventSource.instances[0];
      source?.emitOpen();
      expect(first.onResync).not.toHaveBeenCalled();
      expect(second.onResync).not.toHaveBeenCalled();

      source?.emitError(FakeEventSource.CLOSED);
      vi.advanceTimersByTime(500);
      FakeEventSource.instances[1]?.emitOpen();

      expect(first.onResync).toHaveBeenCalledExactlyOnceWith();
      expect(second.onResync).toHaveBeenCalledExactlyOnceWith();
      firstSubscription.release();
      secondSubscription.release();
      vi.advanceTimersByTime(30_000);
    });

    it('ignores foreground signals for an OPEN source and reopens a non-OPEN source once', () => {
      const broker = createWorkspaceLiveUpdatesBroker();
      const subscriber = handlers();

      const subscription = broker.acquire('acme', subscriber);
      const source = FakeEventSource.instances[0];
      source?.emitOpen();

      window.dispatchEvent(new Event('focus'));
      window.dispatchEvent(new Event('online'));
      vi.advanceTimersByTime(299);
      expect(subscriber.onResync).not.toHaveBeenCalled();
      vi.advanceTimersByTime(1);
      expect(subscriber.onResync).not.toHaveBeenCalled();
      expect(FakeEventSource.instances).toHaveLength(1);

      if (source !== undefined) source.readyState = FakeEventSource.CLOSED;
      Object.defineProperty(document, 'visibilityState', { configurable: true, value: 'visible' });
      document.dispatchEvent(new Event('visibilitychange'));
      window.dispatchEvent(new Event('focus'));
      vi.advanceTimersByTime(300);

      expect(FakeEventSource.instances).toHaveLength(2);
      FakeEventSource.instances[1]?.emitOpen();
      expect(subscriber.onResync).toHaveBeenCalledExactlyOnceWith();
      subscription.release();
      vi.advanceTimersByTime(30_000);
    });

    it('registers foreground listeners once and removes them after idle teardown', () => {
      const addDocumentListener = vi.spyOn(document, 'addEventListener');
      const removeDocumentListener = vi.spyOn(document, 'removeEventListener');
      const addWindowListener = vi.spyOn(window, 'addEventListener');
      const removeWindowListener = vi.spyOn(window, 'removeEventListener');
      const broker = createWorkspaceLiveUpdatesBroker();
      const subscription = broker.acquire('acme', handlers());

      expect(addDocumentListener).toHaveBeenCalledWith('visibilitychange', expect.any(Function));
      expect(addWindowListener).toHaveBeenCalledWith('focus', expect.any(Function));
      expect(addWindowListener).toHaveBeenCalledWith('online', expect.any(Function));
      subscription.release();
      vi.advanceTimersByTime(30_000);

      expect(removeDocumentListener).toHaveBeenCalledWith('visibilitychange', expect.any(Function));
      expect(removeWindowListener).toHaveBeenCalledWith('focus', expect.any(Function));
      expect(removeWindowListener).toHaveBeenCalledWith('online', expect.any(Function));

      window.dispatchEvent(new Event('focus'));
      vi.advanceTimersByTime(300);
      expect(FakeEventSource.instances).toHaveLength(1);
    });

    it('does not let pending recovery or foreground debounce work affect a replacement workspace', () => {
      vi.spyOn(Math, 'random').mockReturnValue(0);
      const broker = createWorkspaceLiveUpdatesBroker();
      const acme = broker.acquire('acme', handlers());
      const staleSource = FakeEventSource.instances[0];

      staleSource?.emitError(FakeEventSource.CLOSED);
      window.dispatchEvent(new Event('focus'));

      const globexHandlers = handlers();
      const globex = broker.acquire('globex', globexHandlers);
      vi.advanceTimersByTime(60_000);

      expect(FakeEventSource.instances).toHaveLength(2);
      expect(FakeEventSource.instances[1]?.closed).toBe(false);
      expect(globexHandlers.onResync).not.toHaveBeenCalled();

      acme.release();
      globex.release();
      vi.advanceTimersByTime(30_000);
    });
  });
});
