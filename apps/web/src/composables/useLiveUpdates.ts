import { onScopeDispose, type Ref, watch } from 'vue';
import { EVENT_TYPES, LIVE_ONLY_EVENT_TYPES, type LiveEnvelope } from '@/lib/eventTypes';

export interface LiveUpdateEvent {
  type: string;
  data: unknown;
  envelope: LiveEnvelope;
}

export interface LiveUpdateHandlers {
  onEvent: (event: LiveUpdateEvent) => void;
  onResync: () => void;
  onReconnectFailed?: () => void;
}

const READY_STATE_OPEN = 1;
const READY_STATE_CLOSED = 2;

const RECONNECT_BASE_DELAY_MS = 1000;
const RECONNECT_BACKOFF_MULTIPLIER = 2;
const RECONNECT_MAX_DELAY_MS = 30000;
const RECONNECT_MAX_ATTEMPTS = 10;

const FOREGROUND_DEBOUNCE_MS = 300;

/**
 * Computes the equal-jitter, capped-exponential backoff delay for a
 * zero-based reconnect attempt: `d = min(base * multiplier^attempt, cap)`,
 * then `delay = d/2 + random() * (d/2)`. Equal jitter spreads reopen attempts
 * out so clients that dropped together (e.g. during a deploy) don't all
 * retry in lockstep against the still-recovering server.
 */
function computeBackoffDelayMs(attempt: number, random: () => number = Math.random): number {
  const uncapped = RECONNECT_BASE_DELAY_MS * RECONNECT_BACKOFF_MULTIPLIER ** attempt;
  const capped = Math.min(uncapped, RECONNECT_MAX_DELAY_MS);
  const half = capped / 2;

  return half + random() * half;
}

/**
 * Coalesces a burst of same-purpose signals (visibility/focus/online can all
 * fire together when a tab regains the foreground) into a single trailing
 * call, `delayMs` after the last signal, so the burst triggers at most one
 * recovery attempt instead of one per signal.
 */
function createTrailingDebouncer(fn: () => void, delayMs: number): { run: () => void; cancel: () => void } {
  let timer: ReturnType<typeof setTimeout> | null = null;

  return {
    run(): void {
      if (timer !== null) clearTimeout(timer);
      timer = setTimeout(() => {
        timer = null;
        fn();
      }, delayMs);
    },
    cancel(): void {
      if (timer !== null) {
        clearTimeout(timer);
        timer = null;
      }
    },
  };
}

/**
 * Owns a single EventSource for the workspace live-event stream
 * (`GET /api/workspaces/{ws}/events`). The `atlas_session` cookie is sent
 * automatically by EventSource, so no auth wiring is needed.
 *
 * The stream is opened when the workspace slug is non-empty, reopened on a slug
 * change, and closed on scope dispose. Every domain message carries its
 * `event_type` as the SSE event name, but the payload is authoritative: each
 * message's `data` is JSON-parsed and `envelope.event_type` drives `onEvent`, so
 * the consumer is robust to framing. An unparseable message is logged at debug
 * and skipped rather than thrown.
 *
 * `onResync` fires on the server's `resync` marker and on every reconnect — but
 * never on the first open, since the view already loaded its own data on mount.
 * It exists to recover events missed while the connection was down.
 *
 * A transient drop leaves the browser's EventSource in `CONNECTING`, which it
 * retries on its own; this composable only takes over once `readyState`
 * reaches `CLOSED`; racing the native retry would open a second socket. It
 * then schedules a reopen with bounded backoff (`computeBackoffDelayMs`) for
 * up to `RECONNECT_MAX_ATTEMPTS` consecutive failures, resets the attempt
 * counter on every successful open, and calls the optional
 * `onReconnectFailed` once the cap is reached without success. A 401/expired
 * session cannot be detected here — `onerror` carries no HTTP status — so it
 * is instead handled by the existing global 401 handler when the reconnect's
 * `onResync` drives a `load()` through the wrapped API client.
 *
 * A backgrounded tab can freeze its timers and sockets without ever firing
 * `onerror`, so the composable also listens for `visibilitychange` (to
 * visible), window `focus`, and `online`, debounced by `FOREGROUND_DEBOUNCE_MS`
 * so a burst of these firing together triggers one recovery, not several. On
 * fire: a non-OPEN stream (CLOSED, or still CONNECTING) is force-reopened
 * through the same `open()` path used by backoff, so the reopen's `onopen`
 * fires the usual non-first resync; an OPEN stream instead fires `onResync()`
 * directly, to recover a zombie socket that looks healthy but stopped
 * delivering events. These listeners are registered only while a stream is
 * open and removed on `close()`/scope dispose, so they never leak or
 * double-register across reopens.
 */
export function useLiveUpdates(wsSlug: Ref<string>, handlers: LiveUpdateHandlers): void {
  let source: EventSource | null = null;
  let firstOpen = true;
  let currentSlug = '';
  let reconnectAttempts = 0;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let foregroundReopenPending = false;

  const dispatch = (event: MessageEvent): void => {
    let parsed: unknown;
    try {
      parsed = JSON.parse(event.data);
    } catch (error) {
      console.debug('useLiveUpdates: ignoring unparseable event', error);
      return;
    }

    if (typeof parsed !== 'object' || parsed === null) {
      console.debug('useLiveUpdates: ignoring event with a non-object payload');
      return;
    }

    const envelope = parsed as LiveEnvelope;
    if (typeof envelope.event_type !== 'string') {
      console.debug('useLiveUpdates: ignoring event without an event_type');
      return;
    }

    handlers.onEvent({ type: envelope.event_type, data: envelope.data, envelope });
  };

  function clearReconnectTimer(): void {
    if (reconnectTimer !== null) {
      clearTimeout(reconnectTimer);
      reconnectTimer = null;
    }
  }

  function runForegroundRecovery(): void {
    if (source === null) return;

    if (source.readyState === READY_STATE_OPEN) {
      handlers.onResync();
      return;
    }

    if (foregroundReopenPending) return;

    foregroundReopenPending = true;
    open(currentSlug, { isReconnect: true });
  }

  const debouncedForegroundRecovery = createTrailingDebouncer(runForegroundRecovery, FOREGROUND_DEBOUNCE_MS);

  function handleForegroundSignal(): void {
    debouncedForegroundRecovery.run();
  }

  function handleVisibilityChange(): void {
    if (document.visibilityState === 'visible') debouncedForegroundRecovery.run();
  }

  function addForegroundListeners(): void {
    document.addEventListener('visibilitychange', handleVisibilityChange);
    window.addEventListener('focus', handleForegroundSignal);
    window.addEventListener('online', handleForegroundSignal);
  }

  function removeForegroundListeners(): void {
    document.removeEventListener('visibilitychange', handleVisibilityChange);
    window.removeEventListener('focus', handleForegroundSignal);
    window.removeEventListener('online', handleForegroundSignal);
  }

  function close(): void {
    clearReconnectTimer();
    debouncedForegroundRecovery.cancel();
    removeForegroundListeners();
    if (source !== null) {
      source.close();
      source = null;
    }
  }

  function scheduleReconnect(): void {
    if (reconnectAttempts >= RECONNECT_MAX_ATTEMPTS) {
      handlers.onReconnectFailed?.();
      return;
    }

    const delay = computeBackoffDelayMs(reconnectAttempts);
    reconnectAttempts += 1;

    reconnectTimer = setTimeout(() => {
      reconnectTimer = null;
      open(currentSlug, { isReconnect: true });
    }, delay);
  }

  function open(ws: string, options: { isReconnect?: boolean } = {}): void {
    close();

    if (typeof EventSource === 'undefined') {
      console.debug('useLiveUpdates: EventSource is unavailable; live updates disabled');
      return;
    }

    currentSlug = ws;
    if (!options.isReconnect) {
      firstOpen = true;
      reconnectAttempts = 0;
      foregroundReopenPending = false;
    }

    const stream = new EventSource(`/api/workspaces/${ws}/events`);
    source = stream;

    stream.onopen = () => {
      reconnectAttempts = 0;
      foregroundReopenPending = false;
      if (firstOpen) {
        firstOpen = false;
        return;
      }
      handlers.onResync();
    };

    // EventSource.readyState follows the spec-defined numeric constants
    // (0=CONNECTING, 1=OPEN, 2=CLOSED); hardcoded here rather than read off
    // `EventSource.CLOSED` since test doubles for EventSource may not define it.
    stream.onerror = () => {
      if (stream.readyState !== READY_STATE_CLOSED) return;
      foregroundReopenPending = false;
      scheduleReconnect();
    };

    // Named domain events are routed by their event_type; the default `message`
    // event is a fallback for a server that omits the name. `resync` is a
    // non-domain marker whose data may not be an envelope. Live-only events (e.g.
    // presence) share the same dispatch path but are catalogued separately.
    for (const type of EVENT_TYPES) {
      stream.addEventListener(type, dispatch as EventListener);
    }
    for (const type of LIVE_ONLY_EVENT_TYPES) {
      stream.addEventListener(type, dispatch as EventListener);
    }
    stream.onmessage = dispatch;
    stream.addEventListener('resync', () => handlers.onResync());

    addForegroundListeners();
  }

  watch(
    wsSlug,
    (ws) => {
      if (ws === '') {
        close();
        return;
      }
      open(ws);
    },
    { immediate: true },
  );

  onScopeDispose(close);
}
