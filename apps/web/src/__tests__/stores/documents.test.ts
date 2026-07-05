import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET, POST, PATCH, DELETE } = vi.hoisted(() => ({
  GET: vi.fn(),
  POST: vi.fn(),
  PATCH: vi.fn(),
  DELETE: vi.fn(),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET, POST, PATCH, DELETE },
}));

import { useDocumentsStore } from '@/stores/documents';

const summary = (id: string, title: string, folderId: string | null = null) => ({
  id,
  title,
  slug: title.toLowerCase(),
  folder_id: folderId,
  head_seq: 1,
  updated_at: '2026-01-01T00:00:00Z',
});

describe('useDocumentsStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('loadSummaries populates document summaries (REQ-W14)', async () => {
    GET.mockResolvedValue({ data: { items: [summary('d1', 'Readme')], has_more: false } });

    const store = useDocumentsStore();
    await store.loadSummaries('ws', 'proj');

    expect(store.summaries).toHaveLength(1);
    expect(store.summaries[0]?.id).toBe('d1');
    expect(store.error).toBeNull();
  });

  it('loadSummaries follows the cursor and accumulates every page', async () => {
    GET.mockResolvedValueOnce({
      data: { items: [summary('d1', 'A')], next_cursor: 'c1', has_more: true },
    }).mockResolvedValueOnce({
      data: { items: [summary('d2', 'B')], next_cursor: null, has_more: false },
    });

    const store = useDocumentsStore();
    await store.loadSummaries('ws', 'proj');

    expect(store.summaries.map((s) => s.id)).toEqual(['d1', 'd2']);
    expect(GET).toHaveBeenCalledTimes(2);
    expect(GET.mock.calls[1]?.[1]?.params?.query?.cursor).toBe('c1');
  });

  it('loadSummaries surfaces the hint on error', async () => {
    GET.mockResolvedValue({ error: { hint: 'denied' } });

    const store = useDocumentsStore();
    await store.loadSummaries('ws', 'proj');

    expect(store.error).toBe('denied');
    expect(store.summaries).toHaveLength(0);
  });

  it('loadSummaries clears stale documents while loading a new project', async () => {
    let resolveLoad: (value: { data: { items: ReturnType<typeof summary>[]; has_more: false } }) => void =
      () => {};
    GET.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveLoad = resolve;
      }),
    );

    const store = useDocumentsStore();
    store.$patch({ summaries: [summary('old', 'Old')] });
    const pending = store.loadSummaries('ws', 'next');

    expect(store.summaries).toHaveLength(0);

    resolveLoad({ data: { items: [summary('new', 'New')], has_more: false } });
    await pending;
    expect(store.summaries[0]?.id).toBe('new');
  });

  it('loadSummaries ignores an older response after a newer load starts', async () => {
    let resolveFirst: (value: { data: { items: ReturnType<typeof summary>[]; has_more: false } }) => void =
      () => {};
    GET.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveFirst = resolve;
      }),
    );
    GET.mockResolvedValueOnce({ data: { items: [summary('new', 'New')], has_more: false } });

    const store = useDocumentsStore();
    const first = store.loadSummaries('ws', 'old');
    await store.loadSummaries('ws', 'new');
    resolveFirst({ data: { items: [summary('old', 'Old')], has_more: false } });
    await first;

    expect(store.summaries[0]?.id).toBe('new');
  });

  it('create refreshes silently without blanking the tree or toggling loading', async () => {
    POST.mockResolvedValueOnce({ data: summary('d2', 'New') });
    let resolveRefresh: (value: { data: { items: ReturnType<typeof summary>[]; has_more: false } }) => void =
      () => {};
    GET.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveRefresh = resolve;
      }),
    );

    const store = useDocumentsStore();
    store.$patch({ summaries: [summary('d1', 'Existing')] });

    const pending = store.create('ws', 'proj', 'New');

    expect(store.summaries).toHaveLength(1);
    expect(store.summaries[0]?.id).toBe('d1');
    expect(store.loading).toBe(false);

    resolveRefresh({ data: { items: [summary('d1', 'Existing'), summary('d2', 'New')], has_more: false } });
    await pending;

    expect(store.summaries).toHaveLength(2);
    expect(store.loading).toBe(false);
  });

  it('loadBacklinks populates backlinks (REQ-W17)', async () => {
    GET.mockResolvedValue({
      data: {
        items: [
          {
            display_title: 'Source',
            source_document_id: 's1',
            source_slug: 'source',
            source_title: 'Source',
          },
        ],
        has_more: false,
      },
    });

    const store = useDocumentsStore();
    await store.loadBacklinks('ws', 'target');

    expect(store.backlinks).toHaveLength(1);
    expect(store.backlinks[0]?.source_slug).toBe('source');
  });

  it('loadBacklinks clears the list on error (never crashes)', async () => {
    GET.mockResolvedValue({ error: { status: 404 } });

    const store = useDocumentsStore();
    await store.loadBacklinks('ws', 'missing');

    expect(store.backlinks).toHaveLength(0);
  });

  it('create returns the new slug and refreshes summaries on success', async () => {
    const created = {
      id: 'd2',
      title: 'New Doc',
      slug: 'new-doc',
      folder_id: null,
      head_revision_id: 'r1',
      head_seq: 1,
      content: '',
      frontmatter: {},
      workspace_id: 'ws',
      created_at: '2026-01-01T00:00:00Z',
      updated_at: '2026-01-01T00:00:00Z',
    };
    POST.mockResolvedValue({ data: created });
    GET.mockResolvedValue({ data: { items: [summary('d2', 'New Doc')], has_more: false } });

    const store = useDocumentsStore();
    const slug = await store.create('ws', 'proj', 'New Doc');

    expect(slug).toBe('new-doc');
    expect(POST).toHaveBeenCalledOnce();
    expect(GET).toHaveBeenCalledOnce();
    expect(store.summaries).toHaveLength(1);
    expect(store.summaries[0]?.id).toBe('d2');
    expect(store.error).toBeNull();
  });

  it('create returns null and sets error from hint on failure', async () => {
    POST.mockResolvedValue({ error: { hint: 'project not found' } });

    const store = useDocumentsStore();
    const slug = await store.create('ws', 'proj', 'Oops');

    expect(slug).toBeNull();
    expect(store.error).toBe('project not found');
    expect(GET).not.toHaveBeenCalled();
  });

  it('rename PATCHes and refreshes summaries', async () => {
    PATCH.mockResolvedValue({ data: {} });
    GET.mockResolvedValue({ data: { items: [summary('d1', 'Renamed')], has_more: false } });

    const store = useDocumentsStore();
    const ok = await store.rename('ws', 'proj', 'my-doc', 'Renamed');

    expect(ok).toBe(true);
    expect(PATCH).toHaveBeenCalledOnce();
    expect(GET).toHaveBeenCalledOnce();
  });

  it('rename returns false and sets error on failure', async () => {
    PATCH.mockResolvedValue({ error: { hint: 'not found' } });

    const store = useDocumentsStore();
    const ok = await store.rename('ws', 'proj', 'bad-slug', 'X');

    expect(ok).toBe(false);
    expect(store.error).toBe('not found');
    expect(GET).not.toHaveBeenCalled();
  });

  it('remove DELETEs and refreshes summaries', async () => {
    DELETE.mockResolvedValue({ data: undefined });
    GET.mockResolvedValue({ data: { items: [], has_more: false } });

    const store = useDocumentsStore();
    const ok = await store.remove('ws', 'proj', 'my-doc');

    expect(ok).toBe(true);
    expect(DELETE).toHaveBeenCalledOnce();
    expect(store.summaries).toHaveLength(0);
  });

  it('remove returns false and sets error on failure', async () => {
    DELETE.mockResolvedValue({ error: { hint: 'forbidden' } });

    const store = useDocumentsStore();
    const ok = await store.remove('ws', 'proj', 'my-doc');

    expect(ok).toBe(false);
    expect(store.error).toBe('forbidden');
    expect(GET).not.toHaveBeenCalled();
  });

  it('move PATCHes the document to a folder and re-fetches', async () => {
    PATCH.mockResolvedValue({ error: undefined });
    GET.mockResolvedValue({ data: { items: [], has_more: false } });

    const store = useDocumentsStore();
    const ok = await store.move('ws', 'proj', 'my-doc', 'folder-1');

    expect(ok).toBe(true);
    expect(PATCH).toHaveBeenCalledWith('/v1/workspaces/{ws}/documents/{slug}/move', {
      params: { path: { ws: 'ws', slug: 'my-doc' } },
      body: { folder_id: 'folder-1' },
    });
  });

  it('move with null folder targets the project root', async () => {
    PATCH.mockResolvedValue({ error: undefined });
    GET.mockResolvedValue({ data: { items: [], has_more: false } });

    const store = useDocumentsStore();
    await store.move('ws', 'proj', 'my-doc', null);

    expect(PATCH).toHaveBeenCalledWith('/v1/workspaces/{ws}/documents/{slug}/move', {
      params: { path: { ws: 'ws', slug: 'my-doc' } },
      body: { folder_id: null },
    });
  });

  it('move returns false and sets error on failure', async () => {
    PATCH.mockResolvedValue({ error: { hint: 'nope' } });

    const store = useDocumentsStore();
    const ok = await store.move('ws', 'proj', 'my-doc', 'folder-1');

    expect(ok).toBe(false);
    expect(store.error).toBe('nope');
    expect(GET).not.toHaveBeenCalled();
  });

  it('copy POSTs to the copy endpoint and re-fetches', async () => {
    POST.mockResolvedValue({ data: { id: 'x', slug: 'my-doc-copy' } });
    GET.mockResolvedValue({ data: { items: [], has_more: false } });

    const store = useDocumentsStore();
    const ok = await store.copy('ws', 'proj', 'my-doc', 'folder-1');

    expect(ok).toBe(true);
    expect(POST).toHaveBeenCalledWith('/v1/workspaces/{ws}/documents/{slug}/copy', {
      params: { path: { ws: 'ws', slug: 'my-doc' } },
      body: { folder_id: 'folder-1' },
    });
  });

  it('copy returns false and sets error on failure', async () => {
    POST.mockResolvedValue({ error: { hint: 'denied' } });

    const store = useDocumentsStore();
    const ok = await store.copy('ws', 'proj', 'my-doc', null);

    expect(ok).toBe(false);
    expect(store.error).toBe('denied');
    expect(GET).not.toHaveBeenCalled();
  });

  const comment = (id: string, body: string, updatedAt = '2026-01-01T00:00:00Z') => ({
    id,
    document_id: 'd1',
    body,
    author: { id: 'u1', type: 'user', display_name: 'Jordan' },
    created_at: '2026-01-01T00:00:00Z',
    updated_at: updatedAt,
  });

  it('loadComments populates the thread and the has-more flag', async () => {
    GET.mockResolvedValue({
      data: { items: [comment('c1', 'First')], next_cursor: 'cur', has_more: true },
    });

    const store = useDocumentsStore();
    await store.loadComments('ws', 'my-doc');

    expect(store.comments.map((c) => c.id)).toEqual(['c1']);
    expect(store.commentsHasMore).toBe(true);
    expect(store.error).toBeNull();
  });

  it('loadComments clears the thread and surfaces the hint on error', async () => {
    GET.mockResolvedValue({ error: { hint: 'forbidden' } });

    const store = useDocumentsStore();
    await store.loadComments('ws', 'my-doc');

    expect(store.comments).toHaveLength(0);
    expect(store.commentsHasMore).toBe(false);
    expect(store.error).toBe('forbidden');
  });

  it('loadMoreComments appends the next page using the stored cursor', async () => {
    GET.mockResolvedValueOnce({
      data: { items: [comment('c1', 'First')], next_cursor: 'cur', has_more: true },
    }).mockResolvedValueOnce({
      data: { items: [comment('c2', 'Second')], next_cursor: null, has_more: false },
    });

    const store = useDocumentsStore();
    await store.loadComments('ws', 'my-doc');
    await store.loadMoreComments('ws', 'my-doc');

    expect(store.comments.map((c) => c.id)).toEqual(['c1', 'c2']);
    expect(store.commentsHasMore).toBe(false);
    expect(GET.mock.calls[1]?.[1]?.params?.query?.cursor).toBe('cur');
  });

  it('addComment appends the created comment only when the thread is fully paged', async () => {
    GET.mockResolvedValue({
      data: { items: [comment('c1', 'First')], next_cursor: null, has_more: false },
    });
    POST.mockResolvedValue({ data: comment('c2', 'Second') });

    const store = useDocumentsStore();
    await store.loadComments('ws', 'my-doc');
    const ok = await store.addComment('ws', 'my-doc', 'Second');

    expect(ok).toBe(true);
    expect(store.comments.map((c) => c.id)).toEqual(['c1', 'c2']);
  });

  it('addComment does not append locally while more pages remain', async () => {
    GET.mockResolvedValue({
      data: { items: [comment('c1', 'First')], next_cursor: 'cur', has_more: true },
    });
    POST.mockResolvedValue({ data: comment('c2', 'Second') });

    const store = useDocumentsStore();
    await store.loadComments('ws', 'my-doc');
    const ok = await store.addComment('ws', 'my-doc', 'Second');

    expect(ok).toBe(true);
    expect(store.comments.map((c) => c.id)).toEqual(['c1']);
  });

  it('addComment returns false and sets error on failure', async () => {
    POST.mockResolvedValue({ error: { hint: 'nope' } });

    const store = useDocumentsStore();
    const ok = await store.addComment('ws', 'my-doc', 'Hi');

    expect(ok).toBe(false);
    expect(store.error).toBe('nope');
  });

  it('removeComment optimistically drops the comment on success', async () => {
    GET.mockResolvedValue({
      data: { items: [comment('c1', 'First'), comment('c2', 'Second')], next_cursor: null, has_more: false },
    });
    DELETE.mockResolvedValue({ error: undefined });

    const store = useDocumentsStore();
    await store.loadComments('ws', 'my-doc');
    const ok = await store.removeComment('ws', 'my-doc', 'c1');

    expect(ok).toBe(true);
    expect(store.comments.map((c) => c.id)).toEqual(['c2']);
  });

  it('removeComment rolls back and sets error on failure', async () => {
    GET.mockResolvedValue({
      data: { items: [comment('c1', 'First'), comment('c2', 'Second')], next_cursor: null, has_more: false },
    });
    DELETE.mockResolvedValue({ error: { hint: 'denied' } });

    const store = useDocumentsStore();
    await store.loadComments('ws', 'my-doc');
    const ok = await store.removeComment('ws', 'my-doc', 'c1');

    expect(ok).toBe(false);
    expect(store.comments.map((c) => c.id)).toEqual(['c1', 'c2']);
    expect(store.error).toBe('denied');
  });

  it('editComment swaps the updated DTO in place', async () => {
    GET.mockResolvedValue({
      data: { items: [comment('c1', 'First')], next_cursor: null, has_more: false },
    });
    PATCH.mockResolvedValue({ data: comment('c1', 'Edited', '2026-02-02T00:00:00Z') });

    const store = useDocumentsStore();
    await store.loadComments('ws', 'my-doc');
    const ok = await store.editComment('ws', 'my-doc', 'c1', 'Edited');

    expect(ok).toBe(true);
    expect(store.comments[0]?.body).toBe('Edited');
    expect(store.comments[0]?.updated_at).toBe('2026-02-02T00:00:00Z');
  });

  it('editComment returns false and sets error on failure', async () => {
    PATCH.mockResolvedValue({ error: { hint: 'not author' } });

    const store = useDocumentsStore();
    const ok = await store.editComment('ws', 'my-doc', 'c1', 'Edited');

    expect(ok).toBe(false);
    expect(store.error).toBe('not author');
  });

  it('uploadAttachment posts raw bytes with the file name header and returns the record', async () => {
    const attachment = { id: 'att-1', document_id: 'd1', file_name: 'shot.png' };
    POST.mockResolvedValue({ data: attachment });

    const file = new File([new Uint8Array([1, 2, 3])], 'shot.png', { type: 'image/png' });
    const store = useDocumentsStore();
    const result = await store.uploadAttachment('ws', 'my-doc', file);

    expect(result).toEqual(attachment);
    expect(store.error).toBeNull();

    const [path, init] = POST.mock.calls[0] ?? [];
    expect(path).toBe('/v1/workspaces/{ws}/documents/{slug}/attachments');
    expect(init.params.path).toEqual({ ws: 'ws', slug: 'my-doc' });
    expect(init.headers['x-file-name']).toBe('shot.png');
    expect(init.headers['Content-Type']).toBe('image/png');
    expect(init.bodySerializer(file)).toBe(file);
  });

  it('uploadAttachment returns null and sets error on failure', async () => {
    POST.mockResolvedValue({ error: { hint: 'too large' } });

    const file = new File([new Uint8Array([1])], 'big.png', { type: 'image/png' });
    const store = useDocumentsStore();
    const result = await store.uploadAttachment('ws', 'my-doc', file);

    expect(result).toBeNull();
    expect(store.error).toBe('too large');
  });
});
