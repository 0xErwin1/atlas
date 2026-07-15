import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ref } from 'vue';

const { GET, POST, DELETE } = vi.hoisted(() => ({
  GET: vi.fn(),
  POST: vi.fn(),
  DELETE: vi.fn(),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET, POST, DELETE },
}));

import { useCommentAttachments } from '@/composables/useCommentAttachments';

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

const entries = ref([{ type: 'comment' as const, comment: { id: 'comment-1' } }]);

describe('useCommentAttachments', () => {
  beforeEach(() => {
    GET.mockReset();
    POST.mockReset();
    DELETE.mockReset();
    entries.value = [{ type: 'comment', comment: { id: 'comment-1' } }];
  });

  it.each([
    ['task', { kind: 'task' as const, ws: 'acme', readableId: 'ATL-1' }],
    ['document', { kind: 'document' as const, ws: 'acme', slug: 'note' }],
  ])('runs the %s lifecycle through the shared target adapter', async (_kind, target) => {
    const currentTarget = ref(target);
    GET.mockResolvedValueOnce({ data: [attachment('existing', 'existing.txt')] }).mockResolvedValueOnce({
      data: new Blob(['download']),
    });
    POST.mockResolvedValueOnce({ data: attachment('new', 'new.txt') });
    DELETE.mockResolvedValueOnce({});

    const attachments = useCommentAttachments(currentTarget, entries);
    await vi.waitFor(() => expect(GET).toHaveBeenCalledTimes(1));
    await attachments.upload('comment-1', new File(['new'], 'new.txt', { type: 'text/plain' }));
    await attachments.download('comment-1', 'new');
    await attachments.delete('comment-1', 'existing');

    expect(attachments.items.value['comment-1']?.map((item) => item.file_name)).toEqual(['new.txt']);
    expect(POST.mock.calls[0]?.[0]).toContain(target.kind === 'task' ? '/tasks/' : '/documents/');
    expect(DELETE.mock.calls[0]?.[0]).toContain(target.kind === 'task' ? '/tasks/' : '/documents/');
  });

  it('excludes a duplicate same-comment upload while preserving an unrelated comment operation', async () => {
    let resolveUpload: (value: unknown) => void = () => {};
    POST.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveUpload = resolve;
      }),
    ).mockResolvedValueOnce({ data: attachment('other', 'other.txt') });
    const currentTarget = ref({ kind: 'task' as const, ws: 'acme', readableId: 'ATL-1' });
    const attachments = useCommentAttachments(currentTarget, entries);

    const first = attachments.upload('comment-1', new File(['first'], 'first.txt'));
    const duplicate = attachments.upload('comment-1', new File(['duplicate'], 'duplicate.txt'));
    const unrelated = attachments.upload('comment-2', new File(['other'], 'other.txt'));

    expect(await duplicate).toBeNull();
    expect(POST).toHaveBeenCalledTimes(2);

    resolveUpload({ data: attachment('first', 'first.txt') });
    await Promise.all([first, unrelated]);
    expect(attachments.items.value['comment-1']?.[0]?.file_name).toBe('first.txt');
    expect(attachments.items.value['comment-2']?.[0]?.file_name).toBe('other.txt');
  });

  it('drops stale list results and clears loading when the workspace parent changes', async () => {
    let resolveTaskList: (value: unknown) => void = () => {};
    GET.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveTaskList = resolve;
      }),
    ).mockResolvedValueOnce({ data: [attachment('document', 'document.txt')] });
    const currentTarget = ref<
      { kind: 'task'; ws: string; readableId: string } | { kind: 'document'; ws: string; slug: string }
    >({
      kind: 'task',
      ws: 'acme',
      readableId: 'ATL-1',
    });
    const attachments = useCommentAttachments(currentTarget, entries);
    await vi.waitFor(() => expect(GET).toHaveBeenCalledTimes(1));

    currentTarget.value = { kind: 'document', ws: 'other', slug: 'note' };
    await vi.waitFor(() => expect(GET).toHaveBeenCalledTimes(2));
    await vi.waitFor(() =>
      expect(attachments.items.value['comment-1']).toEqual([
        expect.objectContaining({ id: 'document', file_name: 'document.txt' }),
      ]),
    );
    resolveTaskList({ data: [attachment('task', 'task.txt')] });
    expect(attachments.items.value['comment-1']).toEqual([
      expect.objectContaining({ id: 'document', file_name: 'document.txt' }),
    ]);
    expect(attachments.isListing('comment-1')).toBe(false);
  });

  it('uses error hints without replacing the last valid list after a failed retry', async () => {
    GET.mockResolvedValueOnce({ data: [attachment('existing', 'existing.txt')] }).mockResolvedValueOnce({
      error: { hint: 'Upload not allowed' },
    });
    const currentTarget = ref({ kind: 'document' as const, ws: 'acme', slug: 'note' });
    const attachments = useCommentAttachments(currentTarget, entries);
    await vi.waitFor(() =>
      expect(attachments.items.value['comment-1']).toEqual([
        expect.objectContaining({ id: 'existing', file_name: 'existing.txt' }),
      ]),
    );
    await attachments.reload('comment-1');

    expect(attachments.items.value['comment-1']).toEqual([
      expect.objectContaining({ id: 'existing', file_name: 'existing.txt' }),
    ]);
    expect(attachments.error.value['comment-1']).toBe('Upload not allowed');
  });
});
