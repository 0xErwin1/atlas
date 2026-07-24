import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ZodType } from 'zod';
import {
  allowResourceCache,
  blockResourceCacheForUnknownAlias,
  configureResourceCacheForTest,
  setResourceCachePrincipal,
} from '@/cache/cacheRuntime';
import {
  buildCacheKey,
  type CacheEnvelope,
  ResourceCache,
  type ResourceCacheStore,
} from '@/cache/resourceCache';

vi.mock('@/api/wrapper', () => ({
  wrappedClient: {
    GET: vi.fn(),
    PUT: vi.fn(),
  },
}));

import { wrappedClient } from '@/api/wrapper';
import { useMarkdownDoc } from '@/composables/useMarkdownDoc';

const mockGet = wrappedClient.GET as ReturnType<typeof vi.fn>;
const mockPut = wrappedClient.PUT as ReturnType<typeof vi.fn>;

const WS = 'acme';
const SLUG = 'my-doc';
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
    async deleteScope(scope) {
      for (const [key, entry] of entries) {
        const matchesPrincipal = key.includes(`|p=${scope.principal}|`);
        const matchesWorkspace = scope.workspaceId === undefined || key.includes(`|w=${scope.workspaceId}|`);
        const matchesTags =
          scope.tagsAny === undefined || entry.tags.some((tag) => scope.tagsAny?.includes(tag));

        if (matchesPrincipal && matchesWorkspace && matchesTags) entries.delete(key);
      }
      return true;
    },
    async clear() {
      return true;
    },
  };
}

beforeEach(() => {
  setActivePinia(createPinia());
  vi.clearAllMocks();
  setResourceCachePrincipal(PRINCIPAL);
  configureResourceCacheForTest(new ResourceCache({ store: cacheStore(new Map()) }));
  allowResourceCache();
});

describe('useMarkdownDoc', () => {
  it('load: returns body and meta after splitting frontmatter', async () => {
    const rawContent = '---\ntitle: My Doc\n---\n\nHello world.';
    mockGet.mockResolvedValue({
      data: {
        slug: SLUG,
        content: rawContent,
        head_revision_id: 'rev-abc',
      },
      error: undefined,
    });

    const { load } = useMarkdownDoc();
    const result = await load(WS, SLUG);

    expect(result.body).toBe('\nHello world.');
    expect(result.meta.title).toBe('My Doc');
    expect(result.headRevisionId).toBe('rev-abc');
  });

  it('load: returns full content as body when no frontmatter', async () => {
    mockGet.mockResolvedValue({
      data: {
        slug: SLUG,
        content: 'No frontmatter here.',
        head_revision_id: 'rev-xyz',
      },
      error: undefined,
    });

    const { load } = useMarkdownDoc();
    const result = await load(WS, SLUG);

    expect(result.body).toBe('No frontmatter here.');
    expect(result.meta).toEqual({});
    expect(result.headRevisionId).toBe('rev-xyz');
  });

  it('load: publishes an exact stale cached body before its matching network refresh resolves', async () => {
    const key = buildCacheKey({
      principal: PRINCIPAL,
      workspaceId: WORKSPACE_ID,
      resourceKind: 'note-body',
      resourceId: SLUG,
    });
    if (key === null) throw new Error('Expected a canonical note body cache key');

    const now = Date.now();
    const entries = new Map<string, CacheEnvelope<unknown>>([
      [
        key,
        {
          schema: 1,
          key,
          payloadVersion: 1,
          storedAt: now,
          validatedAt: now - 120_000,
          lastAccessedAt: now,
          retentionExpiresAt: now + 60_000,
          bytes: 128,
          stale: false,
          tags: [`document:${SLUG}`],
          payload: {
            id: 'doc-1',
            body: 'Cached body',
            meta: { title: 'Cached title' },
            headRevisionId: 'cached-revision',
            slug: SLUG,
          },
        },
      ],
    ]);
    configureResourceCacheForTest(new ResourceCache({ store: cacheStore(entries) }));
    allowResourceCache();
    mockGet.mockResolvedValue({
      data: { id: 'doc-1', slug: SLUG, content: 'Fresh body', head_revision_id: 'fresh-revision' },
      error: undefined,
    });
    const cached: unknown[] = [];

    const { load } = useMarkdownDoc();
    const result = await load(WS, SLUG, {
      workspaceId: WORKSPACE_ID,
      onCached: (document) => cached.push(document),
    });

    expect(cached).toEqual([
      expect.objectContaining({ body: 'Cached body', headRevisionId: 'cached-revision' }),
      expect.objectContaining({ body: 'Fresh body', headRevisionId: 'fresh-revision' }),
    ]);
    expect(result).toMatchObject({ body: 'Fresh body', headRevisionId: 'fresh-revision' });
    expect(mockGet).toHaveBeenCalledOnce();
  });

  it('load: shares one same-key network result across concurrent callers', async () => {
    let resolveNetwork:
      | ((value: {
          data: { id: string; slug: string; content: string; head_revision_id: string };
          error: undefined;
        }) => void)
      | undefined;
    mockGet.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveNetwork = resolve;
      }),
    );
    const firstCached = vi.fn();
    const secondCached = vi.fn();
    const { load } = useMarkdownDoc();

    const first = load(WS, SLUG, { workspaceId: WORKSPACE_ID, onCached: firstCached });
    const second = load(WS, SLUG, { workspaceId: WORKSPACE_ID, onCached: secondCached });

    expect(mockGet).toHaveBeenCalledOnce();
    resolveNetwork?.({
      data: { id: 'doc-1', slug: SLUG, content: 'Shared body', head_revision_id: 'shared-revision' },
      error: undefined,
    });

    await expect(Promise.all([first, second])).resolves.toEqual([
      expect.objectContaining({ body: 'Shared body', headRevisionId: 'shared-revision' }),
      expect.objectContaining({ body: 'Shared body', headRevisionId: 'shared-revision' }),
    ]);
    expect(mockGet).toHaveBeenCalledOnce();
  });

  it('load: reuses a fresh cached body when revisiting a document', async () => {
    mockGet
      .mockResolvedValueOnce({
        data: { id: 'doc-1', slug: SLUG, content: 'Document A', head_revision_id: 'revision-a' },
        error: undefined,
      })
      .mockResolvedValueOnce({
        data: { id: 'doc-2', slug: 'other-doc', content: 'Document B', head_revision_id: 'revision-b' },
        error: undefined,
      });
    const { load } = useMarkdownDoc();

    await load(WS, SLUG, { workspaceId: WORKSPACE_ID, onCached: vi.fn() });
    await load(WS, 'other-doc', { workspaceId: WORKSPACE_ID, onCached: vi.fn() });
    const revisited = await load(WS, SLUG, { workspaceId: WORKSPACE_ID, onCached: vi.fn() });

    expect(revisited).toMatchObject({ body: 'Document A', headRevisionId: 'revision-a' });
    expect(mockGet).toHaveBeenCalledTimes(2);
  });

  it('load: keeps a cached body eligible for an active recovery retry after its refresh fails', async () => {
    const key = buildCacheKey({
      principal: PRINCIPAL,
      workspaceId: WORKSPACE_ID,
      resourceKind: 'note-body',
      resourceId: SLUG,
    });
    if (key === null) throw new Error('Expected a canonical note body cache key');

    const now = Date.now();
    const entries = new Map<string, CacheEnvelope<unknown>>([
      [
        key,
        {
          schema: 1,
          key,
          payloadVersion: 1,
          storedAt: now,
          validatedAt: now - 120_000,
          lastAccessedAt: now,
          retentionExpiresAt: now + 60_000,
          bytes: 128,
          stale: false,
          tags: [`document:${SLUG}`],
          payload: {
            id: 'doc-1',
            body: 'Cached body',
            meta: {},
            headRevisionId: 'cached-revision',
            slug: SLUG,
          },
        },
      ],
    ]);
    const cache = new ResourceCache({ store: cacheStore(entries) });
    configureResourceCacheForTest(cache);
    allowResourceCache();
    mockGet.mockResolvedValueOnce({ error: { title: 'Offline' } }).mockResolvedValueOnce({
      data: { id: 'doc-1', slug: SLUG, content: 'Recovered body', head_revision_id: 'recovered-revision' },
    });
    const cached: unknown[] = [];

    const { load } = useMarkdownDoc();
    await expect(
      load(WS, SLUG, { workspaceId: WORKSPACE_ID, onCached: (document) => cached.push(document) }),
    ).rejects.toThrow('Offline');

    await cache.retry(key);

    expect(cached).toEqual([
      expect.objectContaining({ body: 'Cached body', headRevisionId: 'cached-revision' }),
      expect.objectContaining({ body: 'Recovered body', headRevisionId: 'recovered-revision' }),
    ]);
    expect(mockGet).toHaveBeenCalledTimes(2);
  });

  it('load: uses the online request when the cache kill switch has blocked cache publication', async () => {
    const cache = new ResourceCache({ store: cacheStore(new Map()) });
    configureResourceCacheForTest(cache);
    allowResourceCache();
    mockGet.mockResolvedValue({
      data: { id: 'doc-1', slug: SLUG, content: 'Online body', head_revision_id: 'online-revision' },
      error: undefined,
    });

    blockResourceCacheForUnknownAlias();

    const { load } = useMarkdownDoc();
    await expect(load(WS, SLUG, { workspaceId: WORKSPACE_ID, onCached: vi.fn() })).resolves.toMatchObject({
      body: 'Online body',
      headRevisionId: 'online-revision',
    });

    expect(mockGet).toHaveBeenCalledOnce();
  });

  it('load: falls back to the network after a failed durable cache write', async () => {
    const entries = new Map<string, CacheEnvelope<unknown>>();
    const failedStore: ResourceCacheStore = {
      async get<T>(key: string, _schema: ZodType<T>): Promise<CacheEnvelope<T> | null> {
        return (entries.get(key) as CacheEnvelope<T> | undefined) ?? null;
      },
      async putMany() {
        return false;
      },
      async deleteMany() {
        return true;
      },
      async clear() {
        return true;
      },
    };
    const firstCache = new ResourceCache({ store: failedStore });
    firstCache.allow();
    configureResourceCacheForTest(firstCache);
    mockGet.mockResolvedValueOnce({
      data: { id: 'doc-1', slug: SLUG, content: 'Online body', head_revision_id: 'online-revision' },
      error: undefined,
    });

    const { load } = useMarkdownDoc();
    await load(WS, SLUG, { workspaceId: WORKSPACE_ID, onCached: vi.fn() });
    expect(entries).toEqual(new Map());

    const restartedCache = new ResourceCache({ store: failedStore });
    restartedCache.allow();
    configureResourceCacheForTest(restartedCache);
    mockGet.mockResolvedValueOnce({
      data: { id: 'doc-1', slug: SLUG, content: 'Reloaded body', head_revision_id: 'reloaded-revision' },
      error: undefined,
    });

    await expect(load(WS, SLUG, { workspaceId: WORKSPACE_ID, onCached: vi.fn() })).resolves.toMatchObject({
      body: 'Reloaded body',
      headRevisionId: 'reloaded-revision',
    });
    expect(mockGet).toHaveBeenCalledTimes(2);
  });

  it('load: throws with the HTTP status from the response, even when the body omits it', async () => {
    mockGet.mockResolvedValue({
      data: undefined,
      error: { title: 'Not Found' },
      response: { status: 404 },
    });

    const { load } = useMarkdownDoc();
    const err = (await load(WS, SLUG).catch((e) => e)) as Error & { status?: number };

    expect(err).toBeInstanceOf(Error);
    expect(err.message).toBe('Not Found');
    expect(err.status).toBe(404);
  });

  it('load: falls back to the body status when there is no response', async () => {
    mockGet.mockResolvedValue({
      data: undefined,
      error: { title: 'Gone', status: 410 },
    });

    const { load } = useMarkdownDoc();
    const err = (await load(WS, SLUG).catch((e) => e)) as Error & { status?: number };

    expect(err.status).toBe(410);
  });

  it('save: joins frontmatter+body, PUTs with base_revision_id, returns the new head', async () => {
    mockPut.mockResolvedValue({ data: { head_revision_id: 'rev-def' }, error: undefined });

    const { save } = useMarkdownDoc();
    const result = await save(WS, SLUG, '\nBody text.', { title: 'My Doc' }, 'rev-abc');

    expect(mockPut).toHaveBeenCalledWith(
      expect.stringContaining('/documents/{slug}/content'),
      expect.objectContaining({
        body: expect.objectContaining({ base_revision_id: 'rev-abc' }),
      }),
    );

    expect(result.kind).toBe('ok');
    // The caller must advance its base to this new revision; otherwise the next
    // save would CAS-conflict against itself even with a single editor.
    if (result.kind === 'ok') {
      expect(result.headRevisionId).toBe('rev-def');
    }
  });

  it('save: invalidates the cached body before a subsequent load', async () => {
    mockGet
      .mockResolvedValueOnce({
        data: { id: 'doc-1', slug: SLUG, content: 'Before save', head_revision_id: 'revision-a' },
        error: undefined,
      })
      .mockResolvedValueOnce({
        data: { id: 'doc-1', slug: SLUG, content: 'After save', head_revision_id: 'revision-b' },
        error: undefined,
      });
    mockPut.mockResolvedValue({ data: { head_revision_id: 'revision-b' }, error: undefined });
    const { load, save } = useMarkdownDoc();

    await load(WS, SLUG, { workspaceId: WORKSPACE_ID, onCached: vi.fn() });
    await expect(save(WS, SLUG, 'After save', {}, 'revision-a', WORKSPACE_ID)).resolves.toEqual({
      kind: 'ok',
      headRevisionId: 'revision-b',
    });
    const reloaded = await load(WS, SLUG, { workspaceId: WORKSPACE_ID, onCached: vi.fn() });

    expect(reloaded).toMatchObject({ body: 'After save', headRevisionId: 'revision-b' });
    expect(mockGet).toHaveBeenCalledTimes(2);
  });

  it('save: returns conflict when PUT returns 409', async () => {
    const conflictPayload = {
      type: 'urn:atlas:error:revision-conflict',
      title: 'Revision conflict',
      status: 409,
      current_revision_id: 'rev-new',
      current_seq: 5,
      base_to_current_patch: '@@ -1 +1 @@\n-old\n+new',
    };

    mockPut.mockResolvedValue({
      data: undefined,
      error: conflictPayload,
    });

    const { save } = useMarkdownDoc();
    const result = await save(WS, SLUG, '\nBody.', {}, 'rev-abc');

    expect(result.kind).toBe('conflict');
    if (result.kind === 'conflict') {
      expect(result.problem.current_revision_id).toBe('rev-new');
      expect(result.problem.base_to_current_patch).toBe('@@ -1 +1 @@\n-old\n+new');
    }
  });

  it('save: returns error when PUT fails with non-409 error', async () => {
    mockPut.mockResolvedValue({
      data: undefined,
      error: {
        type: 'urn:atlas:error:not-found',
        title: 'Not Found',
        status: 404,
        hint: 'Document not found',
      },
    });

    const { save } = useMarkdownDoc();
    const result = await save(WS, SLUG, 'Body.', {}, 'rev-abc');

    expect(result.kind).toBe('error');
    if (result.kind === 'error') {
      expect(result.hint).toBe('Document not found');
    }
  });
});
