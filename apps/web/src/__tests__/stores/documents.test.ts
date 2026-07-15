import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ZodType } from 'zod';
import {
  allowResourceCache,
  configureResourceCacheForTest,
  setResourceCachePrincipal,
} from '@/cache/cacheRuntime';
import {
  buildCacheKey,
  type CacheEnvelope,
  ResourceCache,
  type ResourceCacheStore,
} from '@/cache/resourceCache';

const { GET, POST, PATCH, DELETE } = vi.hoisted(() => ({
  GET: vi.fn(),
  POST: vi.fn(),
  PATCH: vi.fn(),
  DELETE: vi.fn(),
}));

const PRINCIPAL = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08';
const WORKSPACE_ID = '019ef171-bbcf-7b90-9be6-5dbb382afd08';

function cacheStore(entries: Map<string, CacheEnvelope<unknown>>): ResourceCacheStore {
  return {
    async get<T>(key: string, _schema: ZodType<T>): Promise<CacheEnvelope<T> | null> {
      const entry = entries.get(key);
      return entry === undefined ? null : (entry as CacheEnvelope<T>);
    },
    async putMany(newEntries) {
      for (const entry of newEntries) entries.set(entry.key, entry);
      return true;
    },
    async deleteMany(keys) {
      for (const key of keys) entries.delete(key);
      return true;
    },
    async clear() {
      return true;
    },
  };
}

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
    setResourceCachePrincipal(PRINCIPAL);
    configureResourceCacheForTest(new ResourceCache({ store: cacheStore(new Map()) }));
    allowResourceCache();
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

  it('keeps concurrent project summary loads isolated by project slug', async () => {
    let resolveAlpha: (value: { data: { items: ReturnType<typeof summary>[]; has_more: false } }) => void =
      () => {};
    GET.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveAlpha = resolve;
      }),
    );
    GET.mockResolvedValueOnce({ data: { items: [summary('b1', 'Beta')], has_more: false } });

    const store = useDocumentsStore();
    const alphaLoad = store.loadSummaries('ws', 'alpha');
    await store.loadSummaries('ws', 'beta');
    resolveAlpha({ data: { items: [summary('a1', 'Alpha')], has_more: false } });
    await alphaLoad;

    expect(store.summariesFor('alpha').map((s) => s.id)).toEqual(['a1']);
    expect(store.summariesFor('beta').map((s) => s.id)).toEqual(['b1']);
    expect(store.summaries.map((s) => s.id)).toEqual(['b1']);
  });

  it('refreshes only the owning project summaries after a mutation', async () => {
    GET.mockResolvedValueOnce({ data: { items: [summary('a1', 'Alpha')], has_more: false } });
    GET.mockResolvedValueOnce({ data: { items: [summary('b1', 'Beta')], has_more: false } });
    PATCH.mockResolvedValueOnce({ data: {} });
    GET.mockResolvedValueOnce({ data: { items: [summary('a1', 'Alpha Renamed')], has_more: false } });

    const store = useDocumentsStore();
    await store.loadSummaries('ws', 'alpha');
    await store.loadSummaries('ws', 'beta');
    const ok = await store.rename('ws', 'alpha', 'alpha', 'Alpha Renamed');

    expect(ok).toBe(true);
    expect(store.summariesFor('alpha').map((s) => s.title)).toEqual(['Alpha Renamed']);
    expect(store.summariesFor('beta').map((s) => s.title)).toEqual(['Beta']);
    expect(store.summaries.map((s) => s.title)).toEqual(['Beta']);
    expect(GET.mock.calls[2]?.[1]?.params?.path?.project_slug).toBe('alpha');
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

  it('preserves the authorized comment backlink source projection', async () => {
    GET.mockResolvedValue({
      data: {
        items: [
          {
            display_title: 'Comment on roadmap',
            source_document_id: 'source',
            source_slug: null,
            source_title: 'Source',
            comment_source: {
              type: 'comment',
              comment_id: 'comment-1',
              parent: { type: 'task', id: 'task-1', readable_id: 'ATL-1', title: 'Roadmap' },
            },
          },
        ],
        has_more: false,
      },
    });

    const store = useDocumentsStore();
    await store.loadBacklinks('ws', 'target', { workspaceId: WORKSPACE_ID });

    expect(store.backlinksStatus).toBe('ready');
    expect(store.backlinks[0]?.source_slug).toBeNull();
    expect(store.backlinks[0]?.comment_source).toMatchObject({
      comment_id: 'comment-1',
      parent: { type: 'task', readable_id: 'ATL-1' },
    });
  });

  it('retains cached backlinks when a refresh fails', async () => {
    GET.mockResolvedValueOnce({
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
    }).mockResolvedValueOnce({ error: { hint: 'offline' } });

    const store = useDocumentsStore();
    await store.loadBacklinks('ws', 'target');
    await store.loadBacklinks('ws', 'target');

    expect(store.backlinks.map((link) => link.source_slug)).toEqual(['source']);
    expect(store.backlinksStatus).toBe('error');
    expect(store.backlinksError).toBe('offline');
  });

  it('hydrates cached backlinks before a failed refresh and recovers them through the active retry', async () => {
    const key = buildCacheKey({
      principal: PRINCIPAL,
      workspaceId: WORKSPACE_ID,
      resourceKind: 'note-secondary',
      resourceId: 'target',
      query: { type: 'backlinks' },
    });
    if (key === null) throw new Error('Expected a canonical backlinks cache key');

    const now = Date.now();
    const entries = new Map<string, CacheEnvelope<unknown>>([
      [
        key,
        {
          schema: 1,
          key,
          payloadVersion: 1,
          storedAt: now,
          validatedAt: now,
          lastAccessedAt: now,
          retentionExpiresAt: now + 60_000,
          bytes: 128,
          stale: false,
          tags: ['document:target', 'secondary:backlinks'],
          payload: [
            {
              display_title: 'Cached source',
              source_document_id: 'cached-source',
              source_slug: 'cached-source',
              source_title: 'Cached source',
            },
          ],
        },
      ],
    ]);
    const cache = new ResourceCache({ store: cacheStore(entries) });
    configureResourceCacheForTest(cache);
    allowResourceCache();
    GET.mockResolvedValueOnce({ error: { hint: 'offline' } }).mockResolvedValueOnce({
      data: {
        items: [
          {
            display_title: 'Recovered source',
            source_document_id: 'recovered-source',
            source_slug: 'recovered-source',
            source_title: 'Recovered source',
          },
        ],
        has_more: false,
      },
    });

    const store = useDocumentsStore();
    await store.loadBacklinks('ws', 'target', { workspaceId: WORKSPACE_ID });

    expect(store.backlinks.map((link) => link.source_slug)).toEqual(['cached-source']);
    expect(store.backlinksStatus).toBe('error');

    await cache.retry(key);

    expect(store.backlinks.map((link) => link.source_slug)).toEqual(['recovered-source']);
    expect(store.backlinksStatus).toBe('ready');
  });

  it('deactivates cached backlink callbacks when switching documents and clearing the target', async () => {
    GET.mockResolvedValue({ data: { items: [], has_more: false } });
    const cache = new ResourceCache({ store: cacheStore(new Map()) });
    const activate = vi.spyOn(cache, 'activate');
    const deactivate = vi.spyOn(cache, 'deactivate');
    configureResourceCacheForTest(cache);
    allowResourceCache();
    const firstKey = buildCacheKey({
      principal: PRINCIPAL,
      workspaceId: WORKSPACE_ID,
      resourceKind: 'note-secondary',
      resourceId: 'first',
      query: { type: 'backlinks' },
    });
    const secondKey = buildCacheKey({
      principal: PRINCIPAL,
      workspaceId: WORKSPACE_ID,
      resourceKind: 'note-secondary',
      resourceId: 'second',
      query: { type: 'backlinks' },
    });
    if (firstKey === null || secondKey === null) throw new Error('Expected canonical backlinks cache keys');

    const store = useDocumentsStore();
    await store.loadBacklinks('ws', 'first', { workspaceId: WORKSPACE_ID });
    await store.loadBacklinks('ws', 'second', { workspaceId: WORKSPACE_ID });
    store.clearSecondaryTarget();

    expect(activate).toHaveBeenCalledTimes(2);
    expect(deactivate).toHaveBeenCalledWith(firstKey);
    expect(deactivate).toHaveBeenCalledWith(secondKey);
  });

  it('deactivates cached backlink callbacks when the workspace changes for the same slug', async () => {
    GET.mockResolvedValue({ data: { items: [], has_more: false } });
    const cache = new ResourceCache({ store: cacheStore(new Map()) });
    const deactivate = vi.spyOn(cache, 'deactivate');
    configureResourceCacheForTest(cache);
    allowResourceCache();
    const firstWorkspaceId = '019ef171-bbcf-7b90-9be6-5dbb382afd09';
    const firstKey = buildCacheKey({
      principal: PRINCIPAL,
      workspaceId: firstWorkspaceId,
      resourceKind: 'note-secondary',
      resourceId: 'shared',
      query: { type: 'backlinks' },
    });
    if (firstKey === null) throw new Error('Expected a canonical backlinks cache key');

    const store = useDocumentsStore();
    await store.loadBacklinks('workspace-a', 'shared', { workspaceId: firstWorkspaceId });
    await store.loadBacklinks('workspace-b', 'shared', { workspaceId: WORKSPACE_ID });

    expect(deactivate).toHaveBeenCalledWith(firstKey);
  });

  it('loadBacklinks clears the list on error (never crashes)', async () => {
    GET.mockResolvedValue({ error: { status: 404 } });

    const store = useDocumentsStore();
    await store.loadBacklinks('ws', 'missing');

    expect(store.backlinks).toHaveLength(0);
  });

  it('loads the default document comment page without a feed selector and rejects a full feed', async () => {
    GET.mockResolvedValueOnce({
      data: {
        items: [{ type: 'comment', comment: comment('comment-1', 'Unexpected full feed') }],
        next_cursor: null,
        has_more: false,
      },
    });

    const store = useDocumentsStore();
    await store.loadComments('ws', 'note');

    expect(GET).toHaveBeenCalledWith('/api/workspaces/{ws}/documents/{slug}/comments', {
      params: { path: { ws: 'ws', slug: 'note' } },
    });
    expect(store.comments).toEqual([]);
    expect(store.commentsStatus).toBe('error');
    expect(store.commentsError).toBe('Received an unsupported full comment feed');
  });

  it('resets note secondary state when the workspace changes for the same slug', async () => {
    GET.mockResolvedValueOnce({
      data: {
        items: [
          {
            display_title: 'Source A',
            source_document_id: 'source-a',
            source_slug: 'source-a',
            source_title: 'Source A',
          },
        ],
        has_more: false,
      },
    }).mockResolvedValueOnce({
      data: { items: [comment('comment-a', 'Comment A')], next_cursor: null, has_more: false },
    });

    const store = useDocumentsStore();
    await store.loadBacklinks('workspace-a', 'note');
    await store.loadComments('workspace-a', 'note');
    store.resetSecondaryTarget('workspace-b', 'note');

    expect(store.backlinks).toEqual([]);
    expect(store.comments).toEqual([]);
    expect(store.backlinksStatus).toBe('idle');
    expect(store.commentsStatus).toBe('idle');
    expect(store.backlinksError).toBeNull();
    expect(store.commentsError).toBeNull();
  });

  it('clears note secondary state when no note target remains selected', async () => {
    GET.mockResolvedValueOnce({
      data: {
        items: [
          {
            display_title: 'Source',
            source_document_id: 'source',
            source_slug: 'source',
            source_title: 'Source',
          },
        ],
        has_more: false,
      },
    }).mockResolvedValueOnce({
      data: { items: [comment('comment', 'Comment')], next_cursor: null, has_more: false },
    });

    const store = useDocumentsStore();
    await store.loadBacklinks('workspace', 'note');
    await store.loadComments('workspace', 'note');
    store.clearSecondaryTarget();

    expect(store.backlinks).toEqual([]);
    expect(store.comments).toEqual([]);
    expect(store.backlinksStatus).toBe('idle');
    expect(store.commentsStatus).toBe('idle');
  });

  it('rejects stale backlinks and comments after a newer target begins', async () => {
    let resolveStaleBacklinks: (value: { data: { items: object[]; has_more: false } }) => void = () => {};
    let resolveStaleComments: (value: {
      data: { items: ReturnType<typeof comment>[]; next_cursor: null; has_more: false };
    }) => void = () => {};
    GET.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveStaleBacklinks = resolve;
      }),
    )
      .mockReturnValueOnce(
        new Promise((resolve) => {
          resolveStaleComments = resolve;
        }),
      )
      .mockResolvedValueOnce({
        data: {
          items: [
            {
              display_title: 'Source B',
              source_document_id: 'source-b',
              source_slug: 'source-b',
              source_title: 'Source B',
            },
          ],
          has_more: false,
        },
      })
      .mockResolvedValueOnce({
        data: { items: [comment('comment-b', 'Comment B')], next_cursor: null, has_more: false },
      });

    const store = useDocumentsStore();
    const staleBacklinks = store.loadBacklinks('workspace-a', 'note-a');
    const staleComments = store.loadComments('workspace-a', 'note-a');

    await store.loadBacklinks('workspace-b', 'note-b');
    await store.loadComments('workspace-b', 'note-b');
    resolveStaleBacklinks({
      data: {
        items: [
          {
            display_title: 'Stale source',
            source_document_id: 'stale-source',
            source_slug: 'stale-source',
            source_title: 'Stale source',
          },
        ],
        has_more: false,
      },
    });
    resolveStaleComments({
      data: { items: [comment('stale-comment', 'Stale comment')], next_cursor: null, has_more: false },
    });
    await Promise.all([staleBacklinks, staleComments]);

    expect(store.backlinks.map((link) => link.source_slug)).toEqual(['source-b']);
    expect(store.comments.map((entry) => entry.id)).toEqual(['comment-b']);
    expect(store.backlinksStatus).toBe('ready');
    expect(store.commentsStatus).toBe('ready');
  });

  it('settles backlinks and comments independently when one secondary request fails', async () => {
    let resolveComments: (value: {
      data: { items: ReturnType<typeof comment>[]; next_cursor: null; has_more: false };
    }) => void = () => {};
    GET.mockResolvedValueOnce({ error: { hint: 'backlinks denied' } }).mockReturnValueOnce(
      new Promise((resolve) => {
        resolveComments = resolve;
      }),
    );

    const store = useDocumentsStore();
    const backlinksLoad = store.loadBacklinks('workspace', 'note');
    const commentsLoad = store.loadComments('workspace', 'note');

    expect(store.backlinksStatus).toBe('pending');
    expect(store.commentsStatus).toBe('pending');

    await backlinksLoad;
    expect(store.backlinksStatus).toBe('error');
    expect(store.backlinksError).toBe('backlinks denied');
    expect(store.commentsStatus).toBe('pending');
    expect(store.commentsError).toBeNull();

    resolveComments({ data: { items: [comment('comment', 'Comment')], next_cursor: null, has_more: false } });
    await commentsLoad;
    expect(store.commentsStatus).toBe('ready');
    expect(store.comments.map((entry) => entry.id)).toEqual(['comment']);
    expect(store.backlinksStatus).toBe('error');
  });

  it('preserves visible secondary data during a same-target refresh', async () => {
    GET.mockResolvedValueOnce({
      data: {
        items: [
          {
            display_title: 'Existing source',
            source_document_id: 'existing-source',
            source_slug: 'existing-source',
            source_title: 'Existing source',
          },
        ],
        has_more: false,
      },
    }).mockResolvedValueOnce({
      data: { items: [comment('existing-comment', 'Existing comment')], next_cursor: null, has_more: false },
    });
    let resolveBacklinks: (value: { data: { items: object[]; has_more: false } }) => void = () => {};
    let resolveComments: (value: {
      data: { items: ReturnType<typeof comment>[]; next_cursor: null; has_more: false };
    }) => void = () => {};
    GET.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveBacklinks = resolve;
      }),
    ).mockReturnValueOnce(
      new Promise((resolve) => {
        resolveComments = resolve;
      }),
    );

    const store = useDocumentsStore();
    await store.loadBacklinks('workspace', 'note');
    await store.loadComments('workspace', 'note');
    const backlinksRefresh = store.loadBacklinks('workspace', 'note');
    const commentsRefresh = store.loadComments('workspace', 'note');

    expect(store.backlinks.map((link) => link.source_slug)).toEqual(['existing-source']);
    expect(store.comments.map((entry) => entry.id)).toEqual(['existing-comment']);
    expect(store.backlinksStatus).toBe('pending');
    expect(store.commentsStatus).toBe('pending');

    resolveBacklinks({ data: { items: [], has_more: false } });
    resolveComments({ data: { items: [], next_cursor: null, has_more: false } });
    await Promise.all([backlinksRefresh, commentsRefresh]);

    expect(store.backlinks).toEqual([]);
    expect(store.comments).toEqual([]);
    expect(store.backlinksStatus).toBe('ready');
    expect(store.commentsStatus).toBe('ready');
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

  it('evicts only the deleted document and project cache tags before returning success', async () => {
    DELETE.mockResolvedValue({ data: undefined });
    GET.mockResolvedValue({ data: { items: [], has_more: false } });
    const purgeTags = vi.fn().mockResolvedValue(true);
    configureResourceCacheForTest({ purgeTags });

    const store = useDocumentsStore();
    const ok = await store.remove('ws', 'proj', 'my-doc', { workspaceId: WORKSPACE_ID });

    expect(ok).toBe(true);
    expect(purgeTags).toHaveBeenCalledWith(['document:my-doc', 'project:proj'], PRINCIPAL, WORKSPACE_ID);
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
    expect(PATCH).toHaveBeenCalledWith('/api/workspaces/{ws}/documents/{slug}/move', {
      params: { path: { ws: 'ws', slug: 'my-doc' } },
      body: { folder_id: 'folder-1' },
    });
  });

  it('move with null folder targets the project root', async () => {
    PATCH.mockResolvedValue({ error: undefined });
    GET.mockResolvedValue({ data: { items: [], has_more: false } });

    const store = useDocumentsStore();
    await store.move('ws', 'proj', 'my-doc', null);

    expect(PATCH).toHaveBeenCalledWith('/api/workspaces/{ws}/documents/{slug}/move', {
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
    expect(POST).toHaveBeenCalledWith('/api/workspaces/{ws}/documents/{slug}/copy', {
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

  it('loadComments clears the thread and surfaces an area-local hint on error', async () => {
    GET.mockResolvedValue({ error: { hint: 'forbidden' } });

    const store = useDocumentsStore();
    await store.loadComments('ws', 'my-doc');

    expect(store.comments).toHaveLength(0);
    expect(store.commentsHasMore).toBe(false);
    expect(store.commentsStatus).toBe('error');
    expect(store.commentsError).toBe('forbidden');
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

  it('does not append a late added comment after navigating to another note', async () => {
    GET.mockResolvedValueOnce({
      data: { items: [comment('comment-a', 'Comment A')], next_cursor: null, has_more: false },
    });
    let resolveAdd: (value: { data: ReturnType<typeof comment> }) => void = () => {};
    POST.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveAdd = resolve;
      }),
    );
    GET.mockResolvedValueOnce({
      data: { items: [comment('comment-b', 'Comment B')], next_cursor: null, has_more: false },
    });

    const store = useDocumentsStore();
    await store.loadComments('workspace', 'note-a');
    const add = store.addComment('workspace', 'note-a', 'Late comment');
    await store.loadComments('workspace', 'note-b');
    resolveAdd({ data: comment('late-comment', 'Late comment') });

    await expect(add).resolves.toBe(true);
    expect(store.comments.map((entry) => entry.id)).toEqual(['comment-b']);
    expect(store.error).toBeNull();
  });

  it('addComment returns false and sets error on failure', async () => {
    GET.mockResolvedValue({
      data: { items: [comment('c1', 'First')], next_cursor: null, has_more: false },
    });
    POST.mockResolvedValue({ error: { hint: 'nope' } });

    const store = useDocumentsStore();
    await store.loadComments('ws', 'my-doc');
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

  it('rolls back a failed deletion after a same-target comments refresh', async () => {
    GET.mockResolvedValueOnce({
      data: { items: [comment('c1', 'First'), comment('c2', 'Second')], next_cursor: null, has_more: false },
    });
    let resolveDelete: (value: { error: { hint: string } }) => void = () => {};
    DELETE.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveDelete = resolve;
      }),
    );
    let resolveRefresh: (value: {
      data: { items: ReturnType<typeof comment>[]; next_cursor: null; has_more: false };
    }) => void = () => {};
    GET.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveRefresh = resolve;
      }),
    );

    const store = useDocumentsStore();
    await store.loadComments('ws', 'my-doc');
    const remove = store.removeComment('ws', 'my-doc', 'c1');
    const refresh = store.loadComments('ws', 'my-doc');
    resolveRefresh({ data: { items: [comment('c2', 'Second')], next_cursor: null, has_more: false } });
    await refresh;
    resolveDelete({ error: { hint: 'denied' } });

    await expect(remove).resolves.toBe(false);
    expect(store.comments.map((entry) => entry.id)).toEqual(['c1', 'c2']);
    expect(store.error).toBe('denied');
  });

  it('rolls back a failed deletion after same-target comment pagination', async () => {
    GET.mockResolvedValueOnce({
      data: { items: [comment('c1', 'First')], next_cursor: 'cursor', has_more: true },
    });
    let resolveDelete: (value: { error: { hint: string } }) => void = () => {};
    DELETE.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveDelete = resolve;
      }),
    );
    let resolvePage: (value: {
      data: { items: ReturnType<typeof comment>[]; next_cursor: null; has_more: false };
    }) => void = () => {};
    GET.mockReturnValueOnce(
      new Promise((resolve) => {
        resolvePage = resolve;
      }),
    );

    const store = useDocumentsStore();
    await store.loadComments('ws', 'my-doc');
    const remove = store.removeComment('ws', 'my-doc', 'c1');
    const page = store.loadMoreComments('ws', 'my-doc');
    resolvePage({ data: { items: [comment('c2', 'Second')], next_cursor: null, has_more: false } });
    await page;
    resolveDelete({ error: { hint: 'denied' } });

    await expect(remove).resolves.toBe(false);
    expect(store.comments.map((entry) => entry.id)).toEqual(['c1']);
    expect(store.error).toBe('denied');
  });

  it('does not roll back a failed deletion or publish its error after navigation', async () => {
    GET.mockResolvedValueOnce({
      data: { items: [comment('comment-a', 'Comment A')], next_cursor: null, has_more: false },
    });
    let resolveDelete: (value: { error: { hint: string } }) => void = () => {};
    DELETE.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveDelete = resolve;
      }),
    );
    GET.mockResolvedValueOnce({
      data: { items: [comment('comment-b', 'Comment B')], next_cursor: null, has_more: false },
    });

    const store = useDocumentsStore();
    await store.loadComments('workspace', 'note-a');
    const remove = store.removeComment('workspace', 'note-a', 'comment-a');
    await store.loadComments('workspace', 'note-b');
    resolveDelete({ error: { hint: 'delete denied' } });

    await expect(remove).resolves.toBe(false);
    expect(store.comments.map((entry) => entry.id)).toEqual(['comment-b']);
    expect(store.error).toBeNull();
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
    GET.mockResolvedValue({
      data: { items: [comment('c1', 'First')], next_cursor: null, has_more: false },
    });
    PATCH.mockResolvedValue({ error: { hint: 'not author' } });

    const store = useDocumentsStore();
    await store.loadComments('ws', 'my-doc');
    const ok = await store.editComment('ws', 'my-doc', 'c1', 'Edited');

    expect(ok).toBe(false);
    expect(store.error).toBe('not author');
  });

  it('does not apply a late edit to another note with the same comment id', async () => {
    GET.mockResolvedValueOnce({
      data: { items: [comment('shared-comment', 'Comment A')], next_cursor: null, has_more: false },
    });
    let resolveEdit: (value: { data: ReturnType<typeof comment> }) => void = () => {};
    PATCH.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveEdit = resolve;
      }),
    );
    GET.mockResolvedValueOnce({
      data: { items: [comment('shared-comment', 'Comment B')], next_cursor: null, has_more: false },
    });

    const store = useDocumentsStore();
    await store.loadComments('workspace', 'note-a');
    const edit = store.editComment('workspace', 'note-a', 'shared-comment', 'Edited A');
    await store.loadComments('workspace', 'note-b');
    resolveEdit({ data: comment('shared-comment', 'Edited A') });

    await expect(edit).resolves.toBe(true);
    expect(store.comments[0]?.body).toBe('Comment B');
    expect(store.error).toBeNull();
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
    expect(path).toBe('/api/workspaces/{ws}/documents/{slug}/attachments');
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
