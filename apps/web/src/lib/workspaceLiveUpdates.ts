import { wrappedClient } from '@/api/wrapper';
import { invalidateLiveResourceCache, resourceCacheEpoch } from '@/cache/cacheRuntime';
import { EVENT_TYPES, LIVE_ONLY_EVENT_TYPES, type LiveEnvelope } from '@/lib/eventTypes';
import { getPlatformTransport, type WorkspaceEventSource } from '@/platform/transport';

const FOREGROUND_DEBOUNCE_MS = 300;
const IDLE_TIMEOUT_MS = 30_000;
const READY_STATE_OPEN = 1;
const READY_STATE_CLOSED = 2;
const RECONNECT_BASE_DELAY_MS = 1_000;
const RECONNECT_MAX_ATTEMPTS = 10;
const RECONNECT_MAX_DELAY_MS = 30_000;

export interface WorkspaceLiveUpdate {
  type: string;
  data: unknown;
  envelope: LiveEnvelope;
}

/**
 * Why a subscriber is being asked to catch up.
 *
 * `reconnect` — the transport reopened after a gap. At most a short window of
 * events was missed, and the cached copy of every resource is still the best
 * base to reconcile against: subscribers revalidate silently and swap the result
 * in atomically, without clearing state or showing a loader. A healthy desktop
 * stream recycles on a cadence, so this path must stay cheap.
 *
 * `desync` — the server's in-process broadcast lagged and dropped events, or a
 * frame arrived that could not be read. What the cache holds may be arbitrarily
 * far behind, so this is the only reason that drops cached state.
 */
export type LiveResyncReason = 'desync' | 'reconnect';

/**
 * What the live stream is doing right now.
 *
 * `offline` is the state that mattered and had no way to reach the UI: the
 * broker gives up after its retry budget, and until this existed the app simply
 * went quiet while looking exactly as live as before.
 */
export type LiveConnectionState = 'connected' | 'reconnecting' | 'offline';

export interface WorkspaceLiveUpdateHandlers {
  onEvent: (update: WorkspaceLiveUpdate) => void;
  onResync: (reason: LiveResyncReason) => void;
  onReconnectFailed?: () => void;
  /**
   * Called with the current state on subscribe, and on every transition after
   * that — a view that mounts mid-outage must not have to wait for the next
   * change to learn it is looking at stale data.
   */
  onConnectionState?: (state: LiveConnectionState) => void;
}

export interface WorkspaceLiveUpdateSubscription {
  release: () => void;
}

export interface WorkspaceLiveUpdatesBroker {
  acquire: (workspaceSlug: string, handlers: WorkspaceLiveUpdateHandlers) => WorkspaceLiveUpdateSubscription;
  setAuthorizationInvalidator: (invalidate: (() => void) | null) => void;
  notifyReconnectFailed: () => void;
  dispose: () => void;
}

type DesktopGateLiveUpdateStatus = 'event' | 'reconnect-failed' | 'reconnected' | 'reconnecting' | 'resync';

interface DesktopGateLiveUpdateObserver {
  recordEvent: (eventType: string, workspaceSlug: string) => void;
  recordStatus: (status: Exclude<DesktopGateLiveUpdateStatus, 'event'>) => void;
}

export interface WorkspaceLiveUpdatesBrokerOptions {
  desktopGateObserver?: DesktopGateLiveUpdateObserver | null;
}

const DESKTOP_GATE_OBSERVER_KEY = Symbol.for('atlas.desktop.gate.live-updates');
const desktopGateObserverRegistry = globalThis as {
  [key: symbol]: DesktopGateLiveUpdateObserver | undefined;
};

type SubscriberId = number;
type Subscriber = WorkspaceLiveUpdateHandlers;

function getDesktopGateLiveUpdateObserver(): DesktopGateLiveUpdateObserver | null {
  return desktopGateObserverRegistry[DESKTOP_GATE_OBSERVER_KEY] ?? null;
}

function isDesktopWorkspaceEventSource(source: WorkspaceEventSource): boolean {
  return (
    (source as WorkspaceEventSource & { atlasDesktopEventSource?: boolean }).atlasDesktopEventSource === true
  );
}

interface Lifetime {
  readonly generation: number;
  readonly cacheEpoch: number;
  readonly workspaceSlug: string;
  source: WorkspaceEventSource;
  sourceToken: symbol;
  readonly subscribers: Map<SubscriberId, Subscriber>;
  idleTimer: ReturnType<typeof setTimeout> | null;
  idleToken: symbol | null;
  reconnectTimer: ReturnType<typeof setTimeout> | null;
  foregroundTimer: ReturnType<typeof setTimeout> | null;
  recoveryAttempt: symbol | null;
  foregroundReopenSourceToken: symbol | null;
  reconnectAttempts: number;
  connectionState: LiveConnectionState;
  firstOpen: boolean;
  listenersInstalled: boolean;
  readonly onForegroundSignal: () => void;
  readonly onVisibilityChange: () => void;
}

function computeBackoffDelayMs(attempt: number): number {
  const capped = Math.min(RECONNECT_BASE_DELAY_MS * 2 ** attempt, RECONNECT_MAX_DELAY_MS);
  return capped / 2 + Math.random() * (capped / 2);
}

export function createWorkspaceLiveUpdatesBroker(
  options: WorkspaceLiveUpdatesBrokerOptions = {},
): WorkspaceLiveUpdatesBroker {
  let lifetime: Lifetime | null = null;
  let generation = 0;
  let nextSubscriberId = 0;
  let authorizationInvalidator: (() => void) | null = null;
  const desktopGateObserver = options.desktopGateObserver ?? getDesktopGateLiveUpdateObserver();

  function isCurrent(candidate: Lifetime, sourceToken?: symbol, recoveryAttempt?: symbol): boolean {
    return (
      lifetime === candidate &&
      lifetime.generation === candidate.generation &&
      candidate.cacheEpoch === resourceCacheEpoch.value &&
      (sourceToken === undefined || candidate.sourceToken === sourceToken) &&
      (recoveryAttempt === undefined || candidate.recoveryAttempt === recoveryAttempt)
    );
  }

  function dispatch(candidate: Lifetime, callback: (subscriber: Subscriber) => void): void {
    const subscriberIds = [...candidate.subscribers.keys()];

    for (const subscriberId of subscriberIds) {
      if (!isCurrent(candidate)) return;

      const subscriber = candidate.subscribers.get(subscriberId);
      if (subscriber === undefined) continue;

      try {
        callback(subscriber);
      } catch (error) {
        console.error('workspaceLiveUpdates: subscriber callback failed', error);
      }
    }
  }

  function beginLiveCacheInvalidation(candidate: Lifetime, envelope?: LiveEnvelope): void {
    void invalidateLiveResourceCache(envelope, candidate.workspaceSlug).catch((error: unknown) => {
      console.error('workspaceLiveUpdates: cache invalidation failed', error);
    });
  }

  function dispatchResync(candidate: Lifetime, reason: LiveResyncReason): void {
    // A benign reconnect deliberately leaves the cache intact: purging it would
    // force every subscriber to refetch from an empty base and show a loader,
    // which is exactly what a periodic upstream recycle must not cost.
    if (reason === 'desync') beginLiveCacheInvalidation(candidate);
    observeDesktopGateStatus(candidate, 'resync');
    dispatch(candidate, (subscriber) => subscriber.onResync(reason));
  }

  /**
   * Reads the resync reason off a `resync` frame. The browser transport carries
   * the server's frame, whose `data` is empty; the desktop transport carries the
   * host's reason. Anything unrecognized is read as `desync`, so an unknown
   * signal degrades to the safe, expensive path rather than being ignored.
   */
  function resyncReasonFrom(event: Event): LiveResyncReason {
    const data = (event as MessageEvent).data;
    if (typeof data !== 'string' || data === '') return 'desync';

    try {
      const parsed: unknown = JSON.parse(data);
      const reason = (parsed as { reason?: unknown } | null)?.reason;
      return reason === 'reconnect' ? 'reconnect' : 'desync';
    } catch {
      return 'desync';
    }
  }

  function observeDesktopGateStatus(
    candidate: Lifetime,
    status: Exclude<DesktopGateLiveUpdateStatus, 'event'>,
  ): void {
    if (desktopGateObserver === null || !isDesktopWorkspaceEventSource(candidate.source)) return;
    desktopGateObserver.recordStatus(status);
  }

  function removeForegroundListeners(candidate: Lifetime): void {
    if (!candidate.listenersInstalled) return;

    document.removeEventListener('visibilitychange', candidate.onVisibilityChange);
    window.removeEventListener('focus', candidate.onForegroundSignal);
    window.removeEventListener('online', candidate.onForegroundSignal);
    candidate.listenersInstalled = false;
  }

  function clearRecoveryWork(candidate: Lifetime): void {
    if (candidate.reconnectTimer !== null) clearTimeout(candidate.reconnectTimer);
    if (candidate.foregroundTimer !== null) clearTimeout(candidate.foregroundTimer);
    candidate.reconnectTimer = null;
    candidate.foregroundTimer = null;
    candidate.recoveryAttempt = null;
  }

  function close(candidate: Lifetime): void {
    if (lifetime !== candidate || lifetime.generation !== candidate.generation) return;

    if (candidate.idleTimer !== null) clearTimeout(candidate.idleTimer);
    candidate.idleTimer = null;
    candidate.idleToken = null;
    clearRecoveryWork(candidate);
    removeForegroundListeners(candidate);
    candidate.source.close();
    lifetime = null;
  }

  function scheduleIdleTeardown(candidate: Lifetime): void {
    if (!isCurrent(candidate) || candidate.subscribers.size !== 0 || candidate.idleTimer !== null) return;

    const idleToken = Symbol('idle');
    candidate.idleToken = idleToken;
    candidate.idleTimer = setTimeout(() => {
      if (!isCurrent(candidate) || candidate.idleToken !== idleToken || candidate.subscribers.size !== 0)
        return;
      close(candidate);
    }, IDLE_TIMEOUT_MS);
  }

  function probeAuthorization(candidate: Lifetime, sourceToken: symbol, recoveryAttempt: symbol): void {
    void wrappedClient
      .GET('/api/v2/acta/workspaces/{ws}', { params: { path: { ws: candidate.workspaceSlug } } })
      .then(({ response }) => {
        if (!isCurrent(candidate, sourceToken, recoveryAttempt)) return;

        if (response.status === 401) {
          close(candidate);
          authorizationInvalidator?.();
          return;
        }

        if (response.status === 403 || response.status === 404) close(candidate);
      })
      .catch(() => {
        if (!isCurrent(candidate, sourceToken, recoveryAttempt)) return;
      });
  }

  /** Records a connection transition and tells the subscribers about it. */
  function setConnectionState(candidate: Lifetime, state: LiveConnectionState): void {
    if (candidate.connectionState === state) return;

    candidate.connectionState = state;
    dispatch(candidate, (subscriber) => subscriber.onConnectionState?.(state));
  }

  function exhaustRecovery(candidate: Lifetime, sourceToken: symbol): void {
    const recoveryAttempt = Symbol('recovery-attempt');
    candidate.recoveryAttempt = recoveryAttempt;
    beginLiveCacheInvalidation(candidate);
    setConnectionState(candidate, 'offline');
    observeDesktopGateStatus(candidate, 'reconnect-failed');
    dispatch(candidate, (subscriber) => subscriber.onReconnectFailed?.());
    probeAuthorization(candidate, sourceToken, recoveryAttempt);
  }

  function scheduleReconnect(candidate: Lifetime, sourceToken: symbol): void {
    if (!isCurrent(candidate, sourceToken) || candidate.reconnectTimer !== null) return;

    if (candidate.reconnectAttempts >= RECONNECT_MAX_ATTEMPTS) {
      exhaustRecovery(candidate, sourceToken);
      return;
    }

    const recoveryAttempt = Symbol('recovery-attempt');
    const delay = computeBackoffDelayMs(candidate.reconnectAttempts);
    candidate.reconnectAttempts += 1;
    candidate.recoveryAttempt = recoveryAttempt;
    setConnectionState(candidate, 'reconnecting');
    observeDesktopGateStatus(candidate, 'reconnecting');
    candidate.reconnectTimer = setTimeout(() => {
      candidate.reconnectTimer = null;
      if (!isCurrent(candidate, sourceToken, recoveryAttempt)) return;
      openSource(candidate, true);
    }, delay);
  }

  function installForegroundListeners(candidate: Lifetime): void {
    if (candidate.listenersInstalled) return;

    document.addEventListener('visibilitychange', candidate.onVisibilityChange);
    window.addEventListener('focus', candidate.onForegroundSignal);
    window.addEventListener('online', candidate.onForegroundSignal);
    candidate.listenersInstalled = true;
  }

  function scheduleForegroundRecovery(candidate: Lifetime): void {
    if (!isCurrent(candidate)) return;

    if (candidate.foregroundTimer !== null) clearTimeout(candidate.foregroundTimer);
    const recoveryAttempt = Symbol('foreground-recovery');
    candidate.recoveryAttempt = recoveryAttempt;
    candidate.foregroundTimer = setTimeout(() => {
      candidate.foregroundTimer = null;
      if (!isCurrent(candidate, undefined, recoveryAttempt)) return;

      if (candidate.source.readyState === READY_STATE_OPEN) return;
      if (candidate.foregroundReopenSourceToken === candidate.sourceToken) return;

      openSource(candidate, true, true);
    }, FOREGROUND_DEBOUNCE_MS);
  }

  function openSource(candidate: Lifetime, isReconnect: boolean, isForegroundReopen = false): void {
    if (!isCurrent(candidate) || typeof EventSource === 'undefined') return;

    if (candidate.reconnectTimer !== null) {
      clearTimeout(candidate.reconnectTimer);
      candidate.reconnectTimer = null;
    }

    candidate.source.close();
    const sourceToken = Symbol('source');
    const source = getPlatformTransport().createWorkspaceEventSource(candidate.workspaceSlug);
    candidate.source = source;
    candidate.sourceToken = sourceToken;
    candidate.recoveryAttempt = null;
    candidate.foregroundReopenSourceToken = isForegroundReopen ? sourceToken : null;

    source.onopen = () => {
      if (!isCurrent(candidate, sourceToken)) return;

      candidate.foregroundReopenSourceToken = null;
      candidate.reconnectAttempts = 0;
      setConnectionState(candidate, 'connected');
      if (candidate.firstOpen) {
        candidate.firstOpen = false;
        return;
      }
      if (isReconnect) {
        observeDesktopGateStatus(candidate, 'reconnected');
        dispatchResync(candidate, 'reconnect');
      }
    };

    source.onerror = () => {
      if (!isCurrent(candidate, sourceToken) || source.readyState !== READY_STATE_CLOSED) return;
      candidate.foregroundReopenSourceToken = null;
      scheduleReconnect(candidate, sourceToken);
    };

    source.onmessage = (event) => dispatchEvent(candidate, sourceToken, event);
    for (const eventType of EVENT_TYPES) {
      source.addEventListener(eventType, (event) =>
        dispatchEvent(candidate, sourceToken, event as MessageEvent),
      );
    }
    for (const eventType of LIVE_ONLY_EVENT_TYPES) {
      source.addEventListener(eventType, (event) =>
        dispatchEvent(candidate, sourceToken, event as MessageEvent),
      );
    }
    source.addEventListener('resync', (event) => {
      if (!isCurrent(candidate, sourceToken)) return;
      dispatchResync(candidate, resyncReasonFrom(event));
    });

    installForegroundListeners(candidate);
  }

  function createLifetime(workspaceSlug: string): Lifetime | null {
    if (typeof EventSource === 'undefined') return null;

    const placeholder = getPlatformTransport().createWorkspaceEventSource(workspaceSlug);
    const candidate: Lifetime = {
      generation: ++generation,
      cacheEpoch: resourceCacheEpoch.value,
      workspaceSlug,
      source: placeholder,
      sourceToken: Symbol('source'),
      subscribers: new Map(),
      idleTimer: null,
      idleToken: null,
      reconnectTimer: null,
      foregroundTimer: null,
      recoveryAttempt: null,
      foregroundReopenSourceToken: null,
      reconnectAttempts: 0,
      // Assumed live until the stream says otherwise: the source is opened
      // immediately after this, and claiming "reconnecting" for that instant
      // would flash a warning on every workspace switch.
      connectionState: 'connected',
      firstOpen: true,
      listenersInstalled: false,
      onForegroundSignal: () => scheduleForegroundRecovery(candidate),
      onVisibilityChange: () => {
        if (document.visibilityState === 'visible') scheduleForegroundRecovery(candidate);
      },
    };

    lifetime = candidate;
    const sourceToken = candidate.sourceToken;
    const source = candidate.source;
    source.onopen = () => {
      if (!isCurrent(candidate, sourceToken)) return;
      candidate.reconnectAttempts = 0;
      candidate.firstOpen = false;
    };
    source.onerror = () => {
      if (!isCurrent(candidate, sourceToken) || source.readyState !== READY_STATE_CLOSED) return;
      scheduleReconnect(candidate, sourceToken);
    };
    source.onmessage = (event) => dispatchEvent(candidate, sourceToken, event);
    for (const eventType of EVENT_TYPES) {
      source.addEventListener(eventType, (event) =>
        dispatchEvent(candidate, sourceToken, event as MessageEvent),
      );
    }
    for (const eventType of LIVE_ONLY_EVENT_TYPES) {
      source.addEventListener(eventType, (event) =>
        dispatchEvent(candidate, sourceToken, event as MessageEvent),
      );
    }
    source.addEventListener('resync', (event) => {
      if (!isCurrent(candidate, sourceToken)) return;
      dispatchResync(candidate, resyncReasonFrom(event));
    });
    installForegroundListeners(candidate);
    return candidate;
  }

  function dispatchEvent(candidate: Lifetime, sourceToken: symbol, event: MessageEvent): void {
    if (!isCurrent(candidate, sourceToken)) return;

    let parsed: unknown;
    try {
      parsed = JSON.parse(event.data);
    } catch (error) {
      console.debug('workspaceLiveUpdates: ignoring unparseable event', error);
      dispatchResync(candidate, 'desync');
      return;
    }

    if (
      typeof parsed !== 'object' ||
      parsed === null ||
      typeof (parsed as LiveEnvelope).event_type !== 'string'
    ) {
      console.debug('workspaceLiveUpdates: ignoring event without an event_type');
      dispatchResync(candidate, 'desync');
      return;
    }

    const envelope = parsed as LiveEnvelope;
    beginLiveCacheInvalidation(candidate, envelope);
    if (desktopGateObserver !== null && isDesktopWorkspaceEventSource(candidate.source)) {
      desktopGateObserver.recordEvent(envelope.event_type, candidate.workspaceSlug);
    }
    dispatch(candidate, (subscriber) =>
      subscriber.onEvent({ type: envelope.event_type, data: envelope.data, envelope }),
    );
  }

  function acquire(
    workspaceSlug: string,
    handlers: WorkspaceLiveUpdateHandlers,
  ): WorkspaceLiveUpdateSubscription {
    if (workspaceSlug === '') return { release: () => {} };

    if (
      lifetime !== null &&
      (lifetime.workspaceSlug !== workspaceSlug || lifetime.cacheEpoch !== resourceCacheEpoch.value)
    ) {
      close(lifetime);
    }

    const candidate = lifetime ?? createLifetime(workspaceSlug);
    if (candidate === null) return { release: () => {} };

    if (candidate.idleTimer !== null) {
      clearTimeout(candidate.idleTimer);
      candidate.idleTimer = null;
      candidate.idleToken = null;
    }

    const subscriberId = ++nextSubscriberId;
    candidate.subscribers.set(subscriberId, handlers);
    handlers.onConnectionState?.(candidate.connectionState);
    let released = false;

    return {
      release(): void {
        if (released || !isCurrent(candidate)) return;
        released = true;
        candidate.subscribers.delete(subscriberId);
        scheduleIdleTeardown(candidate);
      },
    };
  }

  return {
    acquire,
    setAuthorizationInvalidator(invalidate): void {
      authorizationInvalidator = invalidate;
    },
    notifyReconnectFailed(): void {
      if (lifetime === null) return;
      dispatch(lifetime, (subscriber) => subscriber.onReconnectFailed?.());
    },
    dispose(): void {
      if (lifetime !== null) close(lifetime);
    },
  };
}

const defaultBroker = createWorkspaceLiveUpdatesBroker();

export function acquireWorkspaceLiveUpdates(
  workspaceSlug: string,
  handlers: WorkspaceLiveUpdateHandlers,
): WorkspaceLiveUpdateSubscription {
  return defaultBroker.acquire(workspaceSlug, handlers);
}

export function setWorkspaceLiveUpdatesAuthorizationInvalidator(invalidate: (() => void) | null): void {
  defaultBroker.setAuthorizationInvalidator(invalidate);
}

export function disposeWorkspaceLiveUpdates(): void {
  defaultBroker.dispose();
}

export function resetWorkspaceLiveUpdatesForTest(): void {
  defaultBroker.dispose();
  defaultBroker.setAuthorizationInvalidator(null);
}
