import { onScopeDispose, type Ref, reactive, ref, watch } from 'vue';
import { wrappedClient } from '@/api';
import { eventString, type LiveEnvelope } from '@/lib/eventTypes';
import { getPlatformTransport } from '@/platform/transport';
import type { ActorDto } from '@/stores/boards';

const PRESENCE_PATH = '/api/workspaces/{ws}/documents/{slug}/presence' as const;

// The server evicts a presence entry after 45s without a heartbeat; a 20s cadence
// keeps it alive with margin for a slow request or a briefly backgrounded tab.
const HEARTBEAT_INTERVAL_MS = 20_000;

export interface DocumentPresence {
  /** The full visible present-set for the currently-open document. */
  actors: ActorDto[];
  /** Applies a `presence.updated` live event scoped to the current document. */
  apply: (envelope: LiveEnvelope) => void;
}

/**
 * Owns the document presence lifecycle for the currently-open note: it heartbeats
 * the viewer in, keeps the reactive present-set fresh, and leaves on navigation,
 * scope dispose, and tab unload — the document-scoped mirror of `useBoardPresence`.
 *
 * A note is addressed by slug in the URL, but `presence.updated` broadcasts are
 * keyed by the document's canonical UUID. Each heartbeat response carries that
 * `document_id`, which is retained so a broadcast is applied only when it matches
 * the open document. Until the first heartbeat resolves, no broadcast is applied.
 *
 * A heartbeat runs immediately once both `ws` and `slug` are non-empty and then
 * every ~20s; each response seeds the visible set so the first heartbeat already
 * reflects everyone present. Changing note or disposing the scope leaves the
 * previous document best-effort, and a keepalive DELETE on `pagehide`/`beforeunload`
 * covers a tab close the async client could not finish. Heartbeat failures are
 * logged at debug and never throw or stop the interval.
 */
export function useDocumentPresence(ws: Ref<string>, slug: Ref<string | null>): DocumentPresence {
  const actors = ref<ActorDto[]>([]);

  let timer: ReturnType<typeof setInterval> | null = null;
  let activeWs: string | null = null;
  let activeSlug: string | null = null;
  // The open document's canonical UUID, learned from the heartbeat response. A
  // broadcast is only applied when its `document_id` matches this value.
  let resolvedDocId: string | null = null;

  async function heartbeat(wsSlug: string, docSlug: string): Promise<void> {
    try {
      const { data } = await wrappedClient.POST(PRESENCE_PATH, {
        params: { path: { ws: wsSlug, slug: docSlug } },
      });

      if (data !== undefined) {
        resolvedDocId = data.document_id;
        actors.value = data.actors;
      }
    } catch (error) {
      console.debug('useDocumentPresence: heartbeat failed', error);
    }
  }

  async function leave(wsSlug: string, docSlug: string): Promise<void> {
    try {
      await wrappedClient.DELETE(PRESENCE_PATH, {
        params: { path: { ws: wsSlug, slug: docSlug } },
      });
    } catch (error) {
      console.debug('useDocumentPresence: leave failed', error);
    }
  }

  // On tab close the async client cannot flush a request; a keepalive DELETE
  // carrying the CSRF header the middleware would otherwise inject lets the
  // browser deliver the leave as the page goes away.
  function leaveOnUnload(): void {
    if (activeWs === null || activeSlug === null) return;

    try {
      if (getPlatformTransport().isDesktop) {
        void leave(activeWs, activeSlug);
        return;
      }

      void fetch(`/api/workspaces/${activeWs}/documents/${activeSlug}/presence`, {
        method: 'DELETE',
        keepalive: true,
        credentials: 'include',
        headers: { 'X-Atlas-CSRF': '1' },
      });
    } catch (error) {
      console.debug('useDocumentPresence: leave beacon failed', error);
    }
  }

  function stop(): void {
    if (timer !== null) {
      clearInterval(timer);
      timer = null;
    }

    activeWs = null;
    activeSlug = null;
    resolvedDocId = null;
  }

  function start(wsSlug: string, docSlug: string): void {
    activeWs = wsSlug;
    activeSlug = docSlug;

    void heartbeat(wsSlug, docSlug);
    timer = setInterval(() => void heartbeat(wsSlug, docSlug), HEARTBEAT_INTERVAL_MS);
  }

  function apply(envelope: LiveEnvelope): void {
    if (resolvedDocId === null) return;
    if (eventString(envelope.data, 'document_id') !== resolvedDocId) return;

    const payload = envelope.data as { actors?: unknown };
    if (Array.isArray(payload.actors)) {
      actors.value = payload.actors as ActorDto[];
    }
  }

  watch(
    [ws, slug],
    ([wsSlug, docSlug], previous) => {
      stop();

      if (previous !== undefined) {
        const [prevWs, prevSlug] = previous;
        if (prevWs != null && prevWs !== '' && prevSlug != null) void leave(prevWs, prevSlug);
      }

      actors.value = [];
      if (wsSlug !== '' && docSlug !== null) start(wsSlug, docSlug);
    },
    { immediate: true },
  );

  if (typeof window !== 'undefined') {
    window.addEventListener('pagehide', leaveOnUnload);
    window.addEventListener('beforeunload', leaveOnUnload);
  }

  onScopeDispose(() => {
    if (typeof window !== 'undefined') {
      window.removeEventListener('pagehide', leaveOnUnload);
      window.removeEventListener('beforeunload', leaveOnUnload);
    }

    if (activeWs !== null && activeSlug !== null) void leave(activeWs, activeSlug);
    stop();
  });

  return reactive({ actors, apply });
}
