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

  it('loadSummaries surfaces the hint on error', async () => {
    GET.mockResolvedValue({ error: { hint: 'denied' } });

    const store = useDocumentsStore();
    await store.loadSummaries('ws', 'proj');

    expect(store.error).toBe('denied');
    expect(store.summaries).toHaveLength(0);
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
});
