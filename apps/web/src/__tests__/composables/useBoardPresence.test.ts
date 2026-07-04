import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { effectScope, nextTick, ref } from 'vue';

const { POST, DELETE } = vi.hoisted(() => ({
  POST: vi.fn(),
  DELETE: vi.fn(),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { POST, DELETE },
}));

import { type BoardPresence, useBoardPresence } from '@/composables/useBoardPresence';
import type { LiveEnvelope } from '@/lib/eventTypes';

const PATH = '/v1/workspaces/{ws}/boards/{board_id}/presence';

const actor = (id: string, type = 'user') => ({ id, type, display_name: id });

function presenceEnvelope(boardId: string, actors: unknown[]): LiveEnvelope {
  return {
    id: 'evt-1',
    event_type: 'presence.updated',
    version: 1,
    source: 'test',
    workspace_id: 'ws-1',
    board_id: boardId,
    occurred_at: '2026-01-01T00:00:00Z',
    actor: { type: 'user', id: 'u1' },
    data: { board_id: boardId, actors },
  };
}

async function flushMicrotasks(): Promise<void> {
  for (let i = 0; i < 4; i += 1) await Promise.resolve();
}

describe('useBoardPresence', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    POST.mockResolvedValue({ data: { actors: [] }, error: undefined });
    DELETE.mockResolvedValue({ data: undefined, error: undefined });
  });

  afterEach(() => {
    vi.clearAllMocks();
    vi.useRealTimers();
  });

  it('heartbeats immediately on start and seeds actors from the response', async () => {
    POST.mockResolvedValueOnce({ data: { actors: [actor('u1'), actor('a1', 'api_key')] }, error: undefined });

    const scope = effectScope();
    let presence!: BoardPresence;
    scope.run(() => {
      presence = useBoardPresence(ref('acme'), ref<string | null>('board-1'));
    });

    expect(POST).toHaveBeenCalledTimes(1);
    expect(POST).toHaveBeenCalledWith(PATH, { params: { path: { ws: 'acme', board_id: 'board-1' } } });

    await flushMicrotasks();
    expect(presence.actors.map((a) => a.id)).toEqual(['u1', 'a1']);

    scope.stop();
  });

  it('repeats the heartbeat on the interval', () => {
    const scope = effectScope();
    scope.run(() => useBoardPresence(ref('acme'), ref<string | null>('board-1')));

    expect(POST).toHaveBeenCalledTimes(1);

    vi.advanceTimersByTime(20_000);
    expect(POST).toHaveBeenCalledTimes(2);

    vi.advanceTimersByTime(20_000);
    expect(POST).toHaveBeenCalledTimes(3);

    scope.stop();
  });

  it('leaves the previous board and starts the new one on a board change', async () => {
    const ws = ref('acme');
    const boardId = ref<string | null>('board-1');
    const scope = effectScope();
    scope.run(() => useBoardPresence(ws, boardId));

    expect(POST).toHaveBeenCalledTimes(1);

    boardId.value = 'board-2';
    await nextTick();

    expect(DELETE).toHaveBeenCalledWith(PATH, { params: { path: { ws: 'acme', board_id: 'board-1' } } });
    expect(POST).toHaveBeenCalledTimes(2);
    expect(POST).toHaveBeenLastCalledWith(PATH, { params: { path: { ws: 'acme', board_id: 'board-2' } } });

    scope.stop();
  });

  it('leaves the current board on scope dispose (unmount)', () => {
    const scope = effectScope();
    scope.run(() => useBoardPresence(ref('acme'), ref<string | null>('board-1')));

    scope.stop();

    expect(DELETE).toHaveBeenCalledWith(PATH, { params: { path: { ws: 'acme', board_id: 'board-1' } } });
  });

  it('applies a matching presence.updated event and ignores others', async () => {
    const scope = effectScope();
    let presence!: BoardPresence;
    scope.run(() => {
      presence = useBoardPresence(ref('acme'), ref<string | null>('board-1'));
    });
    await flushMicrotasks();

    presence.apply(presenceEnvelope('board-1', [actor('u9')]));
    expect(presence.actors.map((a) => a.id)).toEqual(['u9']);

    presence.apply(presenceEnvelope('other-board', [actor('zzz')]));
    expect(presence.actors.map((a) => a.id)).toEqual(['u9']);

    scope.stop();
  });

  it('keeps the interval alive after a failed heartbeat', async () => {
    vi.spyOn(console, 'debug').mockImplementation(() => {});
    POST.mockRejectedValueOnce(new Error('network down'));

    const scope = effectScope();
    scope.run(() => useBoardPresence(ref('acme'), ref<string | null>('board-1')));

    expect(POST).toHaveBeenCalledTimes(1);
    await flushMicrotasks();

    vi.advanceTimersByTime(20_000);
    expect(POST).toHaveBeenCalledTimes(2);

    scope.stop();
  });
});
