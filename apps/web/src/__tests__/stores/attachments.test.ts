import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET, PATCH, DELETE } = vi.hoisted(() => ({
  GET: vi.fn(),
  PATCH: vi.fn(),
  DELETE: vi.fn(),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET, PATCH, DELETE },
}));

import { useAttachmentsStore, type WorkspaceAttachment } from '@/stores/attachments';

function attachment(overrides: Partial<WorkspaceAttachment> = {}): WorkspaceAttachment {
  return {
    id: 'a1',
    file_name: 'policy.pdf',
    content_type: 'application/pdf',
    size_bytes: 12,
    sha256: 'abc',
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    content_url: '/api/v2/acta/workspaces/acme/attachments/a1',
    owner: { kind: 'document', title: 'Runbook', document_slug: 'runbook' },
    ...overrides,
  } as WorkspaceAttachment;
}

function page(items: WorkspaceAttachment[], hasMore = false, nextCursor?: string) {
  return { data: { items, has_more: hasMore, next_cursor: nextCursor } };
}

describe('useAttachmentsStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('sends the name and owner filters to the API and keeps the returned page', async () => {
    GET.mockResolvedValue(page([attachment()]));
    const store = useAttachmentsStore();

    await store.load('acme', { query: '  policy  ', owner: 'task', type: 'all' });

    expect(GET).toHaveBeenCalledWith('/api/v2/acta/workspaces/{ws}/attachments', {
      params: { path: { ws: 'acme' }, query: { limit: 50, q: 'policy', owner: 'task' } },
    });
    expect(store.items).toHaveLength(1);
    expect(store.error).toBeNull();
  });

  it('omits filters that impose no restriction', async () => {
    GET.mockResolvedValue(page([]));
    const store = useAttachmentsStore();

    await store.load('acme', { query: '', owner: 'all', type: 'all' });

    expect(GET).toHaveBeenCalledWith('/api/v2/acta/workspaces/{ws}/attachments', {
      params: { path: { ws: 'acme' }, query: { limit: 50 } },
    });
  });

  it('asks the API for the image prefix and drops images for the document bucket', async () => {
    GET.mockResolvedValue(
      page([
        attachment({ id: 'img', content_type: 'image/png' }),
        attachment({ id: 'doc', content_type: 'application/pdf' }),
      ]),
    );
    const store = useAttachmentsStore();

    await store.load('acme', { query: '', owner: 'all', type: 'image' });
    expect(GET).toHaveBeenCalledWith('/api/v2/acta/workspaces/{ws}/attachments', {
      params: { path: { ws: 'acme' }, query: { limit: 50, content_type: 'image/' } },
    });
    expect(store.items.map((item) => item.id)).toEqual(['img']);

    await store.load('acme', { query: '', owner: 'all', type: 'other' });
    expect(store.items.map((item) => item.id)).toEqual(['doc']);
  });

  it('appends the next page and replaces on a fresh load', async () => {
    GET.mockResolvedValueOnce(page([attachment({ id: 'a1' })], true, 'cursor-1'));
    const store = useAttachmentsStore();
    await store.load('acme');

    GET.mockResolvedValueOnce(page([attachment({ id: 'a2' })]));
    await store.loadMore('acme');
    expect(store.items.map((item) => item.id)).toEqual(['a1', 'a2']);

    GET.mockResolvedValueOnce(page([attachment({ id: 'a3' })]));
    await store.load('acme');
    expect(store.items.map((item) => item.id)).toEqual(['a3']);
  });

  it('replaces the renamed row in place so the listing keeps its order', async () => {
    GET.mockResolvedValue(page([attachment({ id: 'a1' }), attachment({ id: 'a2' })]));
    const store = useAttachmentsStore();
    await store.load('acme');

    PATCH.mockResolvedValue({ data: attachment({ id: 'a1', file_name: 'new.pdf' }) });
    const renamed = await store.rename('acme', 'a1', 'new.pdf');

    expect(renamed).toBe(true);
    expect(PATCH).toHaveBeenCalledWith('/api/v2/acta/workspaces/{ws}/attachments/{attachment_id}', {
      params: { path: { ws: 'acme', attachment_id: 'a1' } },
      body: { file_name: 'new.pdf' },
    });
    expect(store.items.map((item) => item.file_name)).toEqual(['new.pdf', 'policy.pdf']);
  });

  it('surfaces the API hint and leaves the row untouched when a rename fails', async () => {
    GET.mockResolvedValue(page([attachment({ id: 'a1' })]));
    const store = useAttachmentsStore();
    await store.load('acme');

    PATCH.mockResolvedValue({ error: { hint: 'another attachment here is already named that' } });
    const renamed = await store.rename('acme', 'a1', 'taken.pdf');

    expect(renamed).toBe(false);
    expect(store.error).toBe('another attachment here is already named that');
    expect(store.items[0]?.file_name).toBe('policy.pdf');
  });

  it('drops the deleted row locally instead of refetching', async () => {
    GET.mockResolvedValue(page([attachment({ id: 'a1' }), attachment({ id: 'a2' })]));
    const store = useAttachmentsStore();
    await store.load('acme');
    GET.mockClear();

    DELETE.mockResolvedValue({});
    const removed = await store.remove('acme', 'a1');

    expect(removed).toBe(true);
    expect(GET).not.toHaveBeenCalled();
    expect(store.items.map((item) => item.id)).toEqual(['a2']);
  });

  it('keeps the row when the delete fails', async () => {
    GET.mockResolvedValue(page([attachment({ id: 'a1' })]));
    const store = useAttachmentsStore();
    await store.load('acme');

    DELETE.mockResolvedValue({ error: { hint: 'nope' } });
    expect(await store.remove('acme', 'a1')).toBe(false);
    expect(store.items).toHaveLength(1);
    expect(store.error).toBe('nope');
  });

  it('ignores a stale response that lands after a newer request', async () => {
    const store = useAttachmentsStore();

    let resolveFirst: (value: unknown) => void = () => {};
    GET.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveFirst = resolve;
      }),
    );
    const first = store.load('acme', { query: 'old', owner: 'all', type: 'all' });

    GET.mockResolvedValueOnce(page([attachment({ id: 'fresh' })]));
    await store.load('acme', { query: 'new', owner: 'all', type: 'all' });

    resolveFirst(page([attachment({ id: 'stale' })]));
    await first;

    expect(store.items.map((item) => item.id)).toEqual(['fresh']);
  });
});
