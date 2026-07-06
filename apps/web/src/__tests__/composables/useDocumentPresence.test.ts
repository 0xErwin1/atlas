import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { effectScope, nextTick, ref } from 'vue';

const { POST, DELETE } = vi.hoisted(() => ({
  POST: vi.fn(),
  DELETE: vi.fn(),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { POST, DELETE },
}));

import { type DocumentPresence, useDocumentPresence } from '@/composables/useDocumentPresence';
import type { LiveEnvelope } from '@/lib/eventTypes';

const PATH = '/api/workspaces/{ws}/documents/{slug}/presence';

const actor = (id: string, type = 'user') => ({ id, type, display_name: id });

function presenceEnvelope(documentId: string, actors: unknown[]): LiveEnvelope {
  return {
    id: 'evt-1',
    event_type: 'presence.updated',
    version: 1,
    source: 'test',
    workspace_id: 'ws-1',
    board_id: null,
    document_id: documentId,
    occurred_at: '2026-01-01T00:00:00Z',
    actor: { type: 'user', id: 'u1' },
    data: { document_id: documentId, actors },
  };
}

async function flushMicrotasks(): Promise<void> {
  for (let i = 0; i < 4; i += 1) await Promise.resolve();
}

describe('useDocumentPresence', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    POST.mockResolvedValue({ data: { document_id: 'doc-uuid-1', actors: [] }, error: undefined });
    DELETE.mockResolvedValue({ data: undefined, error: undefined });
  });

  afterEach(() => {
    vi.clearAllMocks();
    vi.useRealTimers();
  });

  it('heartbeats immediately on start and seeds actors from the response', async () => {
    POST.mockResolvedValueOnce({
      data: { document_id: 'doc-uuid-1', actors: [actor('u1'), actor('a1', 'api_key')] },
      error: undefined,
    });

    const scope = effectScope();
    let presence!: DocumentPresence;
    scope.run(() => {
      presence = useDocumentPresence(ref('acme'), ref<string | null>('my-note'));
    });

    expect(POST).toHaveBeenCalledTimes(1);
    expect(POST).toHaveBeenCalledWith(PATH, { params: { path: { ws: 'acme', slug: 'my-note' } } });

    await flushMicrotasks();
    expect(presence.actors.map((a) => a.id)).toEqual(['u1', 'a1']);

    scope.stop();
  });

  it('repeats the heartbeat on the interval', () => {
    const scope = effectScope();
    scope.run(() => useDocumentPresence(ref('acme'), ref<string | null>('my-note')));

    expect(POST).toHaveBeenCalledTimes(1);

    vi.advanceTimersByTime(20_000);
    expect(POST).toHaveBeenCalledTimes(2);

    vi.advanceTimersByTime(20_000);
    expect(POST).toHaveBeenCalledTimes(3);

    scope.stop();
  });

  it('leaves the previous note and starts the new one on a slug change', async () => {
    const ws = ref('acme');
    const slug = ref<string | null>('note-1');
    const scope = effectScope();
    scope.run(() => useDocumentPresence(ws, slug));

    expect(POST).toHaveBeenCalledTimes(1);

    slug.value = 'note-2';
    await nextTick();

    expect(DELETE).toHaveBeenCalledWith(PATH, { params: { path: { ws: 'acme', slug: 'note-1' } } });
    expect(POST).toHaveBeenCalledTimes(2);
    expect(POST).toHaveBeenLastCalledWith(PATH, { params: { path: { ws: 'acme', slug: 'note-2' } } });

    scope.stop();
  });

  it('leaves the current note on scope dispose (unmount)', () => {
    const scope = effectScope();
    scope.run(() => useDocumentPresence(ref('acme'), ref<string | null>('note-1')));

    scope.stop();

    expect(DELETE).toHaveBeenCalledWith(PATH, { params: { path: { ws: 'acme', slug: 'note-1' } } });
  });

  it('applies a broadcast matching the resolved document id and ignores others', async () => {
    const scope = effectScope();
    let presence!: DocumentPresence;
    scope.run(() => {
      presence = useDocumentPresence(ref('acme'), ref<string | null>('my-note'));
    });
    // Let the first heartbeat resolve the canonical document id used for matching.
    await flushMicrotasks();

    presence.apply(presenceEnvelope('doc-uuid-1', [actor('u9')]));
    expect(presence.actors.map((a) => a.id)).toEqual(['u9']);

    presence.apply(presenceEnvelope('other-doc', [actor('zzz')]));
    expect(presence.actors.map((a) => a.id)).toEqual(['u9']);

    scope.stop();
  });

  it('ignores broadcasts that arrive before the first heartbeat resolves', () => {
    // A never-resolving heartbeat leaves the document id unknown, so no broadcast is
    // applied — the client must not adopt a present-set it cannot attribute.
    POST.mockReturnValueOnce(new Promise(() => {}));

    const scope = effectScope();
    let presence!: DocumentPresence;
    scope.run(() => {
      presence = useDocumentPresence(ref('acme'), ref<string | null>('my-note'));
    });

    presence.apply(presenceEnvelope('doc-uuid-1', [actor('u9')]));
    expect(presence.actors).toEqual([]);

    scope.stop();
  });

  it('keeps the interval alive after a failed heartbeat', async () => {
    vi.spyOn(console, 'debug').mockImplementation(() => {});
    POST.mockRejectedValueOnce(new Error('network down'));

    const scope = effectScope();
    scope.run(() => useDocumentPresence(ref('acme'), ref<string | null>('my-note')));

    expect(POST).toHaveBeenCalledTimes(1);
    await flushMicrotasks();

    vi.advanceTimersByTime(20_000);
    expect(POST).toHaveBeenCalledTimes(2);

    scope.stop();
  });
});
