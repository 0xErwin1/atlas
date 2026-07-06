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
 */
export function useLiveUpdates(wsSlug: Ref<string>, handlers: LiveUpdateHandlers): void {
  let source: EventSource | null = null;
  let firstOpen = true;

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

  function close(): void {
    if (source !== null) {
      source.close();
      source = null;
    }
  }

  function open(ws: string): void {
    close();

    if (typeof EventSource === 'undefined') {
      console.debug('useLiveUpdates: EventSource is unavailable; live updates disabled');
      return;
    }

    firstOpen = true;
    const stream = new EventSource(`/api/workspaces/${ws}/events`);
    source = stream;

    stream.onopen = () => {
      if (firstOpen) {
        firstOpen = false;
        return;
      }
      handlers.onResync();
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
