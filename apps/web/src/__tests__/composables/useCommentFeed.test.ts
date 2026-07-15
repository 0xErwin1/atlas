import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET, POST, DELETE } = vi.hoisted(() => ({
  GET: vi.fn(),
  POST: vi.fn(),
  DELETE: vi.fn(),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET, POST, DELETE },
}));

import { normalizeCommentLinkTarget, useCommentFeed } from '@/composables/useCommentFeed';

const comment = (id: string, body: string) => ({
  id,
  task_id: 'task-id',
  body,
  author: { id: 'author', type: 'user', display_name: 'Author' },
  created_at: `2026-01-01T00:00:0${id}Z`,
  updated_at: `2026-01-01T00:00:0${id}Z`,
});

describe('useCommentFeed', () => {
  beforeEach(() => {
    GET.mockReset();
    POST.mockReset();
    DELETE.mockReset();
  });

  it('normalizes unavailable targets to the exact no-disclosure label', () => {
    expect(normalizeCommentLinkTarget({ status: 'unavailable', label: 'leaked' })).toEqual({
      status: 'unavailable',
      label: 'Recurso no disponible',
    });
    expect(normalizeCommentLinkTarget({ status: 'available', id: 'task-1', type: 'task' })).toEqual({
      status: 'available',
      id: 'task-1',
      type: 'task',
    });
  });

  it('loads and merges paginated full task feeds without changing the legacy store API', async () => {
    GET.mockResolvedValueOnce({
      data: {
        items: [
          {
            type: 'comment',
            comment: comment('1', 'First'),
            links: [{ target: { status: 'unavailable', label: 'not safe', id: 'hidden' } }],
          },
        ],
        next_cursor: 'cursor-1',
        has_more: true,
      },
    }).mockResolvedValueOnce({
      data: {
        items: [
          { type: 'comment', comment: comment('1', 'First'), links: [] },
          {
            type: 'event',
            id: 'event-2',
            comment_id: '1',
            kind: 'link_added',
            created_at: '2026-01-01T00:00:02Z',
            target: { status: 'unavailable', label: 'leaked', id: 'hidden' },
          },
        ],
        next_cursor: null,
        has_more: false,
      },
    });

    const feed = useCommentFeed();
    const target = { kind: 'task' as const, ws: 'acme', readableId: 'ATL-1' };
    await feed.load(target);
    await feed.loadMore(target);

    expect(GET.mock.calls[0]?.[1]?.params?.query).toEqual({ feed: 'full' });
    expect(GET.mock.calls[1]?.[1]?.params?.query).toEqual({ feed: 'full', cursor: 'cursor-1' });
    expect(feed.entries.value).toHaveLength(2);
    expect(feed.entries.value[0]).toMatchObject({
      type: 'comment',
      links: [{ target: { label: 'Recurso no disponible' } }],
    });
    expect(feed.entries.value[1]).toMatchObject({
      type: 'event',
      target: { status: 'unavailable', label: 'Recurso no disponible' },
    });
    expect(feed.hasMore.value).toBe(false);
    expect(feed.status.value).toBe('ready');
  });

  it('resets for a new document target and ignores a stale task response', async () => {
    let resolveTask: (value: unknown) => void = () => {};
    GET.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveTask = resolve;
      }),
    ).mockResolvedValueOnce({
      data: { items: [{ type: 'comment', comment: comment('2', 'Document'), links: [] }], has_more: false },
    });

    const feed = useCommentFeed();
    const task = { kind: 'task' as const, ws: 'acme', readableId: 'ATL-1' };
    const document = { kind: 'document' as const, ws: 'other', slug: 'note' };
    const pending = feed.load(task);
    await feed.load(document);
    resolveTask({
      data: { items: [{ type: 'comment', comment: comment('1', 'Stale'), links: [] }], has_more: false },
    });
    await pending;

    expect(feed.entries.value).toMatchObject([{ type: 'comment', comment: { body: 'Document' } }]);
    expect(feed.error.value).toBeNull();
  });

  it('surfaces full-feed errors and does not retain stale entries', async () => {
    GET.mockResolvedValue({ error: { hint: 'No permitido' } });

    const feed = useCommentFeed();
    await feed.load({ kind: 'document', ws: 'acme', slug: 'private' });

    expect(feed.entries.value).toEqual([]);
    expect(feed.status.value).toBe('error');
    expect(feed.error.value).toBe('No permitido');
  });
});
