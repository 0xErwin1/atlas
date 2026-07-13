import { wrappedClient } from '@/api/wrapper';
import { EVENT_TYPES, LIVE_ONLY_EVENT_TYPES, type LiveEnvelope } from '@/lib/eventTypes';

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

export interface WorkspaceLiveUpdateHandlers {
  onEvent: (update: WorkspaceLiveUpdate) => void;
  onResync: () => void;
  onReconnectFailed?: () => void;
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

type SubscriberId = number;
type Subscriber = WorkspaceLiveUpdateHandlers;

interface Lifetime {
  readonly generation: number;
  readonly workspaceSlug: string;
  source: EventSource;
  sourceToken: symbol;
  readonly subscribers: Map<SubscriberId, Subscriber>;
  idleTimer: ReturnType<typeof setTimeout> | null;
  idleToken: symbol | null;
  reconnectTimer: ReturnType<typeof setTimeout> | null;
  foregroundTimer: ReturnType<typeof setTimeout> | null;
  recoveryAttempt: symbol | null;
  foregroundReopenSourceToken: symbol | null;
  reconnectAttempts: number;
  firstOpen: boolean;
  listenersInstalled: boolean;
  readonly onForegroundSignal: () => void;
  readonly onVisibilityChange: () => void;
}

function computeBackoffDelayMs(attempt: number): number {
  const capped = Math.min(RECONNECT_BASE_DELAY_MS * 2 ** attempt, RECONNECT_MAX_DELAY_MS);
  return capped / 2 + Math.random() * (capped / 2);
}

export function createWorkspaceLiveUpdatesBroker(): WorkspaceLiveUpdatesBroker {
  let lifetime: Lifetime | null = null;
  let generation = 0;
  let nextSubscriberId = 0;
  let authorizationInvalidator: (() => void) | null = null;

  function isCurrent(candidate: Lifetime, sourceToken?: symbol, recoveryAttempt?: symbol): boolean {
    return (
      lifetime === candidate &&
      lifetime.generation === candidate.generation &&
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
    if (!isCurrent(candidate)) return;

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
      .GET('/api/workspaces/{ws}', { params: { path: { ws: candidate.workspaceSlug } } })
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

  function exhaustRecovery(candidate: Lifetime, sourceToken: symbol): void {
    const recoveryAttempt = Symbol('recovery-attempt');
    candidate.recoveryAttempt = recoveryAttempt;
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
    const source = new EventSource(`/api/workspaces/${candidate.workspaceSlug}/events`);
    candidate.source = source;
    candidate.sourceToken = sourceToken;
    candidate.recoveryAttempt = null;
    candidate.foregroundReopenSourceToken = isForegroundReopen ? sourceToken : null;

    source.onopen = () => {
      if (!isCurrent(candidate, sourceToken)) return;

      candidate.foregroundReopenSourceToken = null;
      candidate.reconnectAttempts = 0;
      if (candidate.firstOpen) {
        candidate.firstOpen = false;
        return;
      }
      if (isReconnect) dispatch(candidate, (subscriber) => subscriber.onResync());
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
    source.addEventListener('resync', () => {
      if (!isCurrent(candidate, sourceToken)) return;
      dispatch(candidate, (subscriber) => subscriber.onResync());
    });

    installForegroundListeners(candidate);
  }

  function createLifetime(workspaceSlug: string): Lifetime | null {
    if (typeof EventSource === 'undefined') return null;

    const placeholder = new EventSource(`/api/workspaces/${workspaceSlug}/events`);
    const candidate: Lifetime = {
      generation: ++generation,
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
    source.addEventListener('resync', () => {
      if (!isCurrent(candidate, sourceToken)) return;
      dispatch(candidate, (subscriber) => subscriber.onResync());
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
      return;
    }

    if (
      typeof parsed !== 'object' ||
      parsed === null ||
      typeof (parsed as LiveEnvelope).event_type !== 'string'
    ) {
      console.debug('workspaceLiveUpdates: ignoring event without an event_type');
      return;
    }

    const envelope = parsed as LiveEnvelope;
    dispatch(candidate, (subscriber) =>
      subscriber.onEvent({ type: envelope.event_type, data: envelope.data, envelope }),
    );
  }

  function acquire(
    workspaceSlug: string,
    handlers: WorkspaceLiveUpdateHandlers,
  ): WorkspaceLiveUpdateSubscription {
    if (workspaceSlug === '') return { release: () => {} };

    if (lifetime !== null && lifetime.workspaceSlug !== workspaceSlug) close(lifetime);

    const candidate = lifetime ?? createLifetime(workspaceSlug);
    if (candidate === null) return { release: () => {} };

    if (candidate.idleTimer !== null) {
      clearTimeout(candidate.idleTimer);
      candidate.idleTimer = null;
      candidate.idleToken = null;
    }

    const subscriberId = ++nextSubscriberId;
    candidate.subscribers.set(subscriberId, handlers);
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
