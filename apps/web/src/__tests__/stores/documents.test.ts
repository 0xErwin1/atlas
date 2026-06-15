import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET } = vi.hoisted(() => ({ GET: vi.fn() }));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET },
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
});
