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

const attachment = (id: string, fileName: string) => ({
  id,
  comment_id: 'comment-1',
  file_name: fileName,
  content_type: 'text/plain',
  size_bytes: 4,
  sha256: 'digest-not-exposed-by-state',
  created_at: '2026-01-01T00:00:00Z',
  actor: null,
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

  it('runs the task comment attachment lifecycle with isolated loading and error state', async () => {
    GET.mockResolvedValueOnce({ data: [attachment('attachment-1', 'existing.txt')] }).mockResolvedValueOnce({
      data: new Blob(['download']),
    });
    POST.mockResolvedValueOnce({ data: attachment('attachment-2', 'new.txt') });
    DELETE.mockResolvedValueOnce({});

    const feed = useCommentFeed();
    const target = { kind: 'task' as const, ws: 'acme', readableId: 'ATL-1' };
    await feed.loadAttachments(target, 'comment-1');
    await feed.uploadAttachment(target, 'comment-1', new File(['new'], 'new.txt', { type: 'text/plain' }));
    const downloaded = await feed.downloadAttachment(target, 'comment-1', 'attachment-2');
    await feed.deleteAttachment(target, 'comment-1', 'attachment-1');

    expect(feed.attachments.value['comment-1']?.map((item) => item.file_name)).toEqual(['new.txt']);
    expect(downloaded).toBeInstanceOf(Blob);
    expect(POST.mock.calls[0]?.[0]).toContain('/tasks/{readable_id}/comments/{comment_id}/attachments');
    expect(feed.attachmentError.value['comment-1']).toBeUndefined();
  });

  it('uploads and deletes document attachments with raw file bytes and canonical headers', async () => {
    GET.mockResolvedValueOnce({ data: [attachment('attachment-1', 'existing.txt')] }).mockResolvedValueOnce({
      error: { hint: 'Download denied' },
    });
    POST.mockResolvedValueOnce({ data: attachment('attachment-2', 'new.txt') });
    DELETE.mockResolvedValueOnce({});

    const feed = useCommentFeed();
    const target = { kind: 'document' as const, ws: 'acme', slug: 'note' };
    await feed.loadAttachments(target, 'comment-1');
    const file = new File(['new'], 'new.txt', { type: 'text/plain' });
    const uploaded = await feed.uploadAttachment(target, 'comment-1', file);
    const downloaded = await feed.downloadAttachment(target, 'comment-1', 'attachment-1');
    const deleted = await feed.deleteAttachment(target, 'comment-1', 'attachment-1');

    expect(GET.mock.calls[0]?.[0]).toContain('/documents/{slug}/comments/{comment_id}/attachments');
    expect(POST.mock.calls[0]?.[0]).toBe(
      '/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments',
    );
    expect(POST.mock.calls[0]?.[1]).toMatchObject({
      body: [110, 101, 119],
      params: { header: { 'x-file-name': 'new.txt' } },
      headers: { 'Content-Type': 'text/plain' },
    });
    expect(POST.mock.calls[0]?.[1]?.bodySerializer([110, 101, 119])).toBe(file);
    expect(uploaded).toMatchObject({ id: 'attachment-2', file_name: 'new.txt' });
    expect(downloaded).toBeNull();
    expect(deleted).toBe(true);
    expect(feed.attachments.value['comment-1']).toEqual([
      expect.objectContaining({ id: 'attachment-2', file_name: 'new.txt' }),
    ]);
    expect(feed.attachmentError.value['comment-1']).toBeUndefined();
  });

  it('clears stale attachment loading state after switching targets', async () => {
    let resolveTask: (value: unknown) => void = () => {};
    GET.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveTask = resolve;
      }),
    ).mockResolvedValueOnce({ data: [attachment('attachment-2', 'document.txt')] });

    const feed = useCommentFeed();
    const task = { kind: 'task' as const, ws: 'acme', readableId: 'ATL-1' };
    const document = { kind: 'document' as const, ws: 'acme', slug: 'note' };
    const stale = feed.loadAttachments(task, 'task-comment');

    await feed.loadAttachments(document, 'document-comment');
    resolveTask({ data: [attachment('attachment-1', 'task.txt')] });
    await stale;

    expect(feed.isAttachmentListLoading('task-comment')).toBe(false);
    expect(feed.isAttachmentListLoading('document-comment')).toBe(false);
    expect(feed.attachments.value['task-comment']).toBeUndefined();
    expect(feed.attachments.value['document-comment']).toEqual([
      expect.objectContaining({ id: 'attachment-2', file_name: 'document.txt' }),
    ]);
  });

  it('does not let an older same-comment list overwrite a newer upload', async () => {
    let resolveList: (value: unknown) => void = () => {};
    GET.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveList = resolve;
      }),
    );
    POST.mockResolvedValueOnce({ data: attachment('attachment-2', 'new.txt') });

    const feed = useCommentFeed();
    const target = { kind: 'task' as const, ws: 'acme', readableId: 'ATL-1' };
    const staleList = feed.loadAttachments(target, 'comment-1');
    await feed.uploadAttachment(target, 'comment-1', new File(['new'], 'new.txt'));
    resolveList({ data: [attachment('attachment-1', 'old.txt')] });
    await staleList;

    expect(feed.attachments.value['comment-1']).toEqual([
      expect.objectContaining({ id: 'attachment-2', file_name: 'new.txt' }),
    ]);
  });
});
