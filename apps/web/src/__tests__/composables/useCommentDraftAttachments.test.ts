import { flushPromises } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ref } from 'vue';
import { useCommentDraftAttachments } from '@/composables/useCommentDraftAttachments';

const { post, remove } = vi.hoisted(() => ({ post: vi.fn(), remove: vi.fn() }));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { POST: post, DELETE: remove },
}));

describe('useCommentDraftAttachments', () => {
  beforeEach(() => {
    post.mockReset();
    remove.mockReset();
  });

  it('creates one task draft for parallel first selections and exposes canonical image Markdown', async () => {
    const target = ref({ kind: 'task' as const, ws: 'acme', readableId: 'ATL-1' });
    const uploads = new Map<string, { url: string; markdown: string }>([
      ['first.png', { url: '/first', markdown: '![first](/first)' }],
      ['second.png', { url: '/second', markdown: '![second](/second)' }],
    ]);
    post.mockImplementation((path: string, options?: { bodySerializer?: () => FormData }) => {
      if (path.endsWith('/comment-drafts')) {
        return Promise.resolve({ data: { id: 'draft-1', expires_at: '2026-07-17T00:00:00Z' } });
      }

      const file = options?.bodySerializer?.().get('file') as File;
      return Promise.resolve({ data: { id: file.name, ...uploads.get(file.name) } });
    });

    const draft = useCommentDraftAttachments(target);
    const first = draft.uploadImage(new File(['one'], 'first.png', { type: 'image/png' }));
    const second = draft.uploadImage(new File(['two'], 'second.png', { type: 'image/png' }));

    await expect(first).resolves.toEqual({ url: '/first', markdown: '![first](/first)' });
    await expect(second).resolves.toEqual({ url: '/second', markdown: '![second](/second)' });
    await flushPromises();

    expect(post).toHaveBeenCalledTimes(3);
    expect(post.mock.calls.filter(([path]) => path.endsWith('/comment-drafts'))).toHaveLength(1);
    expect(draft.draftId.value).toBe('draft-1');
    expect(draft.attachments.value.map((attachment) => attachment.status)).toEqual(['uploaded', 'uploaded']);
  });

  it('retries a failed document upload and cancels the retained draft only when explicitly discarded', async () => {
    const target = ref({ kind: 'document' as const, ws: 'acme', slug: 'note' });
    post
      .mockResolvedValueOnce({ data: { id: 'draft-2', expires_at: '2026-07-17T00:00:00Z' } })
      .mockResolvedValueOnce({ error: { detail: 'offline' } })
      .mockResolvedValueOnce({ data: { id: 'file-1', url: '/file-1', markdown: '[notes](/file-1)' } });
    remove.mockResolvedValue({});

    const draft = useCommentDraftAttachments(target);
    await expect(draft.enqueue(new File(['note'], 'notes.txt', { type: 'text/plain' }))).resolves.toBeNull();

    const failed = draft.attachments.value[0];
    expect(failed?.status).toBe('error');
    expect(draft.draftId.value).toBe('draft-2');
    await expect(draft.retry(failed?.clientId ?? '')).resolves.toEqual({
      url: '/file-1',
      markdown: '[notes](/file-1)',
    });
    expect(draft.attachments.value[0]?.status).toBe('uploaded');

    await expect(draft.discard()).resolves.toBe(true);
    expect(remove).toHaveBeenCalledWith(
      '/api/workspaces/{ws}/documents/{slug}/comment-drafts/{draft_id}',
      expect.objectContaining({ params: { path: { ws: 'acme', slug: 'note', draft_id: 'draft-2' } } }),
    );
    expect(draft.draftId.value).toBeNull();
    expect(draft.attachments.value).toEqual([]);
  });

  it('removes an uploaded task attachment through the canonical draft-compatible comment route', async () => {
    const target = ref({ kind: 'task' as const, ws: 'acme', readableId: 'ATL-1' });
    post
      .mockResolvedValueOnce({ data: { id: 'draft-3', expires_at: '2026-07-17T00:00:00Z' } })
      .mockResolvedValueOnce({ data: { id: 'attachment-3', url: '/file', markdown: '[file](/file)' } });
    remove.mockResolvedValue({});

    const draft = useCommentDraftAttachments(target);
    await draft.enqueue(new File(['file'], 'file.txt', { type: 'text/plain' }));
    const clientId = draft.attachments.value[0]?.clientId ?? '';

    await expect(draft.remove(clientId)).resolves.toBe(true);
    expect(remove).toHaveBeenCalledWith(
      '/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}',
      expect.objectContaining({
        params: {
          path: { ws: 'acme', readable_id: 'ATL-1', comment_id: 'draft-3', attachment_id: 'attachment-3' },
        },
      }),
    );
    expect(draft.attachments.value).toEqual([]);
  });

  it('compensates for an upload that succeeds after its attachment was removed', async () => {
    const target = ref({ kind: 'task' as const, ws: 'acme', readableId: 'ATL-1' });
    let resolveUpload: ((value: { data: { id: string; url: string; markdown: string } }) => void) | undefined;
    const upload = new Promise<{ data: { id: string; url: string; markdown: string } }>((resolve) => {
      resolveUpload = resolve;
    });
    post
      .mockResolvedValueOnce({ data: { id: 'draft-4', expires_at: '2026-07-17T00:00:00Z' } })
      .mockReturnValueOnce(upload);
    remove.mockResolvedValue({});

    const draft = useCommentDraftAttachments(target);
    const queuedUpload = draft.enqueue(new File(['file'], 'late.txt', { type: 'text/plain' }));
    await flushPromises();
    const clientId = draft.attachments.value[0]?.clientId ?? '';
    const removal = draft.remove(clientId);

    if (resolveUpload === undefined) throw new Error('Upload did not start');
    resolveUpload({ data: { id: 'attachment-4', url: '/late', markdown: '[late](/late)' } });

    await expect(queuedUpload).resolves.toBeNull();
    await expect(removal).resolves.toBe(true);
    expect(remove).toHaveBeenCalledWith(
      '/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}',
      expect.objectContaining({
        params: {
          path: { ws: 'acme', readable_id: 'ATL-1', comment_id: 'draft-4', attachment_id: 'attachment-4' },
        },
      }),
    );
    expect(draft.attachments.value).toEqual([]);
  });

  it('replays a lost upload response before deleting a removed attachment', async () => {
    const target = ref({ kind: 'task' as const, ws: 'acme', readableId: 'ATL-1' });
    let rejectUpload: ((reason: Error) => void) | undefined;
    const upload = new Promise<never>((_resolve, reject) => {
      rejectUpload = reject;
    });
    post
      .mockResolvedValueOnce({ data: { id: 'draft-4b', expires_at: '2026-07-17T00:00:00Z' } })
      .mockReturnValueOnce(upload)
      .mockResolvedValueOnce({ data: { id: 'attachment-4b', url: '/late', markdown: '[late](/late)' } });
    remove.mockResolvedValue({});

    const draft = useCommentDraftAttachments(target);
    const queuedUpload = draft.enqueue(new File(['file'], 'lost.txt', { type: 'text/plain' }));
    await flushPromises();
    const clientId = draft.attachments.value[0]?.clientId ?? '';
    const removal = draft.remove(clientId);

    if (rejectUpload === undefined) throw new Error('Upload did not start');
    rejectUpload(new Error('response lost after commit'));

    await expect(queuedUpload).resolves.toBeNull();
    await expect(removal).resolves.toBe(true);
    const uploadCalls = post.mock.calls.filter(([path]) => path.endsWith('/attachments'));
    expect(uploadCalls).toHaveLength(2);
    expect(uploadCalls[0]?.[1]?.params.header['x-upload-token']).toBe(
      uploadCalls[1]?.[1]?.params.header['x-upload-token'],
    );
    expect(remove).toHaveBeenCalledWith(
      '/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}',
      expect.objectContaining({
        params: {
          path: { ws: 'acme', readable_id: 'ATL-1', comment_id: 'draft-4b', attachment_id: 'attachment-4b' },
        },
      }),
    );
    expect(draft.attachments.value).toEqual([]);
  });

  it('recovers from a rejected draft creation and reuses stable create and upload tokens on retry', async () => {
    const target = ref({ kind: 'task' as const, ws: 'acme', readableId: 'ATL-1' });
    post
      .mockRejectedValueOnce(new Error('connection lost'))
      .mockResolvedValueOnce({ data: { id: 'draft-5', expires_at: '2026-07-17T00:00:00Z' } })
      .mockRejectedValueOnce(new Error('response lost after commit'))
      .mockResolvedValueOnce({ data: { id: 'attachment-5', url: '/retry', markdown: '[retry](/retry)' } });

    const draft = useCommentDraftAttachments(target);
    await expect(draft.enqueue(new File(['file'], 'retry.txt', { type: 'text/plain' }))).resolves.toBeNull();

    const failed = draft.attachments.value[0];
    expect(failed?.status).toBe('error');
    expect(failed?.error).toContain('Failed to create attachment draft');

    await expect(draft.retry(failed?.clientId ?? '')).resolves.toBeNull();
    await expect(draft.retry(failed?.clientId ?? '')).resolves.toEqual({
      url: '/retry',
      markdown: '[retry](/retry)',
    });

    const createCalls = post.mock.calls.filter(([path]) => path.endsWith('/comment-drafts'));
    const uploadCalls = post.mock.calls.filter(([path]) => path.endsWith('/attachments'));
    expect(createCalls).toHaveLength(2);
    expect(uploadCalls).toHaveLength(2);
    expect(createCalls[0]?.[1]?.params.header['x-create-token']).toBe(
      createCalls[1]?.[1]?.params.header['x-create-token'],
    );
    expect(uploadCalls[0]?.[1]?.params.header['x-upload-token']).toBe(
      uploadCalls[1]?.[1]?.params.header['x-upload-token'],
    );
    expect(draft.attachments.value[0]?.status).toBe('uploaded');
  });
});
