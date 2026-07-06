import { onScopeDispose, type Ref, reactive, ref, watch } from 'vue';
import { wrappedClient } from '@/api';
import { eventString, type LiveEnvelope } from '@/lib/eventTypes';
import type { ActorDto } from '@/stores/boards';

const PRESENCE_PATH = '/api/workspaces/{ws}/boards/{board_id}/presence' as const;

// The server evicts a presence entry after 45s without a heartbeat; a 20s cadence
// keeps it alive with margin for a slow request or a briefly backgrounded tab.
const HEARTBEAT_INTERVAL_MS = 20_000;

export interface BoardPresence {
  /** The full visible present-set for the currently-viewed board. */
  actors: ActorDto[];
  /** Applies a `presence.updated` live event scoped to the current board. */
  apply: (envelope: LiveEnvelope) => void;
}

/**
 * Owns the board presence lifecycle for the currently-viewed board: it heartbeats
 * the viewer in, keeps the reactive present-set fresh, and leaves on navigation,
 * scope dispose, and tab unload.
 *
 * A heartbeat runs immediately once both `ws` and `boardId` are non-empty and then
 * every ~20s; each response seeds the visible set so the first heartbeat already
 * reflects everyone present. Changing board or disposing the scope leaves the
 * previous board best-effort, and a keepalive DELETE on `pagehide`/`beforeunload`
 * covers a tab close the async client could not finish. Heartbeat failures are
 * logged at debug and never throw or stop the interval.
 */
export function useBoardPresence(ws: Ref<string>, boardId: Ref<string | null>): BoardPresence {
  const actors = ref<ActorDto[]>([]);

  let timer: ReturnType<typeof setInterval> | null = null;
  let activeWs: string | null = null;
  let activeBoard: string | null = null;

  async function heartbeat(wsSlug: string, board: string): Promise<void> {
    try {
      const { data } = await wrappedClient.POST(PRESENCE_PATH, {
        params: { path: { ws: wsSlug, board_id: board } },
      });

      if (data !== undefined) actors.value = data.actors;
    } catch (error) {
      console.debug('useBoardPresence: heartbeat failed', error);
    }
  }

  async function leave(wsSlug: string, board: string): Promise<void> {
    try {
      await wrappedClient.DELETE(PRESENCE_PATH, {
        params: { path: { ws: wsSlug, board_id: board } },
      });
    } catch (error) {
      console.debug('useBoardPresence: leave failed', error);
    }
  }

  // On tab close the async client cannot flush a request; a keepalive DELETE
  // carrying the CSRF header the middleware would otherwise inject lets the
  // browser deliver the leave as the page goes away.
  function leaveOnUnload(): void {
    if (activeWs === null || activeBoard === null) return;

    try {
      void fetch(`/api/workspaces/${activeWs}/boards/${activeBoard}/presence`, {
        method: 'DELETE',
        keepalive: true,
        credentials: 'include',
        headers: { 'X-Atlas-CSRF': '1' },
      });
    } catch (error) {
      console.debug('useBoardPresence: leave beacon failed', error);
    }
  }

  function stop(): void {
    if (timer !== null) {
      clearInterval(timer);
      timer = null;
    }

    activeWs = null;
    activeBoard = null;
  }

  function start(wsSlug: string, board: string): void {
    activeWs = wsSlug;
    activeBoard = board;

    void heartbeat(wsSlug, board);
    timer = setInterval(() => void heartbeat(wsSlug, board), HEARTBEAT_INTERVAL_MS);
  }

  function apply(envelope: LiveEnvelope): void {
    if (eventString(envelope.data, 'board_id') !== boardId.value) return;

    const payload = envelope.data as { actors?: unknown };
    if (Array.isArray(payload.actors)) {
      actors.value = payload.actors as ActorDto[];
    }
  }

  watch(
    [ws, boardId],
    ([wsSlug, board], previous) => {
      stop();

      if (previous !== undefined) {
        const [prevWs, prevBoard] = previous;
        if (prevWs != null && prevWs !== '' && prevBoard != null) void leave(prevWs, prevBoard);
      }

      actors.value = [];
      if (wsSlug !== '' && board !== null) start(wsSlug, board);
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

    if (activeWs !== null && activeBoard !== null) void leave(activeWs, activeBoard);
    stop();
  });

  return reactive({ actors, apply });
}
