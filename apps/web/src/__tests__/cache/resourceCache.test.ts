import { describe, expect, it } from 'vitest';
import { z } from 'zod';
import {
  AUTHORIZATION_LEASE_MS,
  buildCacheKey,
  createCacheEnvelopeSchema,
  DEFAULT_CACHE_POLICY,
  ResourceCache,
  type ResourceCacheStore,
  startHydrationAndRevalidation,
} from '@/cache/resourceCache';

const workspaceId = '018f8e6d-7c15-7c72-8a41-2f5295e0c0f1';
const principal = 'user:018f8e6d-7c15-7c72-8a41-2f5295e0c0f2';

describe('resource cache contracts', () => {
  it('builds distinct canonical keys for every authorized resource scope', () => {
    const base = {
      principal,
      workspaceId,
      resourceKind: 'task-list' as const,
      resourceId: 'workspace-tasks',
      query: {
        archived: false,
        labels: ['urgent', 'bug'],
      },
      setValuedQueryKeys: ['labels'],
    };

    expect(buildCacheKey(base)).toBe(
      `v1|p=${principal}|w=${workspaceId}|k=task-list|r=workspace-tasks|q={"archived":false,"labels":["bug","urgent"]}`,
    );
    expect(buildCacheKey({ ...base, principal: 'api_key:018f8e6d-7c15-7c72-8a41-2f5295e0c0f3' })).not.toBe(
      buildCacheKey(base),
    );
    expect(buildCacheKey({ ...base, workspaceId: '018f8e6d-7c15-7c72-8a41-2f5295e0c0f4' })).not.toBe(
      buildCacheKey(base),
    );
    expect(buildCacheKey({ ...base, resourceKind: 'task-detail', resourceId: 'workspace-tasks' })).not.toBe(
      buildCacheKey(base),
    );
    expect(buildCacheKey({ ...base, resourceId: 'another-resource' })).not.toBe(buildCacheKey(base));
    expect(buildCacheKey({ ...base, query: { archived: true, labels: ['bug', 'urgent'] } })).not.toBe(
      buildCacheKey(base),
    );
  });

  it('fails closed for noncanonical identities', () => {
    for (const invalidIdentity of [
      { principal: '', workspaceId },
      { principal: ` ${principal}`, workspaceId },
      { principal: 'user:not-a-uuid', workspaceId },
      { principal, workspaceId: ` ${workspaceId}` },
      { principal, workspaceId: workspaceId.toUpperCase() },
    ]) {
      expect(
        buildCacheKey({
          ...invalidIdentity,
          resourceKind: 'note-body',
          resourceId: 'note-a',
        }),
      ).toBeNull();
    }

    expect(AUTHORIZATION_LEASE_MS).toBe(24 * 60 * 60 * 1000);
    expect(DEFAULT_CACHE_POLICY.persistent.maxBytes).toBe(50 * 1024 * 1024);
  });

  it('rejects credential-bearing payloads at the cache envelope boundary', () => {
    const schema = createCacheEnvelopeSchema(z.object({ title: z.string() }).passthrough());

    expect(
      schema.safeParse({
        schema: 1,
        key: buildCacheKey({ principal, workspaceId, resourceKind: 'note-body', resourceId: 'note-a' }),
        payloadVersion: 1,
        storedAt: 1,
        validatedAt: 1,
        lastAccessedAt: 1,
        retentionExpiresAt: 2,
        bytes: 24,
        stale: false,
        tags: ['note:note-a'],
        payload: { title: 1 },
      }).success,
    ).toBe(false);
    expect(
      schema.safeParse({
        schema: 1,
        key: buildCacheKey({ principal, workspaceId, resourceKind: 'note-body', resourceId: 'note-a' }),
        payloadVersion: 1,
        storedAt: 1,
        validatedAt: 1,
        lastAccessedAt: 1,
        retentionExpiresAt: 2,
        bytes: 24,
        stale: false,
        tags: ['note:note-a'],
        payload: { title: 'note', authorization: 'Bearer secret' },
      }).success,
    ).toBe(false);
    expect(
      schema.safeParse({
        schema: 1,
        key: buildCacheKey({ principal, workspaceId, resourceKind: 'note-body', resourceId: 'note-a' }),
        payloadVersion: 1,
        storedAt: 1,
        validatedAt: 1,
        lastAccessedAt: 1,
        retentionExpiresAt: 2,
        bytes: 24,
        stale: false,
        tags: ['note:note-a'],
        payload: { title: 'note', attachmentBytes: new Uint8Array([1, 2, 3]) },
      }).success,
    ).toBe(false);
  });
});

describe('resource cache revalidation payload delivery', () => {
  function noopStore(): ResourceCacheStore {
    return {
      get: async () => null,
      putMany: async () => true,
      deleteMany: async () => true,
      clear: async () => true,
    };
  }

  it('hands back the fetched payload when the generation is bumped mid-flight', async () => {
    const cache = new ResourceCache({ store: noopStore(), policy: DEFAULT_CACHE_POLICY });
    cache.allow();

    const key = buildCacheKey({ principal, workspaceId, resourceKind: 'note-body', resourceId: 'note-a' });
    if (key === null) throw new Error('expected a canonical cache key');

    const request = {
      key,
      payloadSchema: z.object({ id: z.string() }),
      tags: ['document:note-a'],
      freshForMs: 1000,
      activeForMs: 2000,
      retentionForMs: 10_000,
      // A purge/block landing while the fetch is in flight bumps the cache
      // generation, so the revalidation resolves into a superseded context.
      load: async () => {
        cache.block();
        return { id: 'doc-1' };
      },
      publish: () => {},
      isCurrent: () => true,
    };

    const result = await startHydrationAndRevalidation(cache, request).completion;

    expect(result.payload).toEqual({ id: 'doc-1' });
  });

  describe('aging a tag instead of dropping it', () => {
    const payloadSchema = z.object({ id: z.string() });

    async function cacheWithBoard(): Promise<{ cache: ResourceCache; key: string }> {
      const cache = new ResourceCache({ store: noopStore(), policy: DEFAULT_CACHE_POLICY });
      cache.allow();

      const key = buildCacheKey({
        principal,
        workspaceId,
        resourceKind: 'task-board',
        resourceId: 'board-a',
      });
      if (key === null) throw new Error('expected a canonical cache key');

      await cache.revalidate({
        key,
        payloadSchema,
        tags: ['board:board-a'],
        freshForMs: 120_000,
        retentionForMs: 600_000,
        load: async () => ({ id: 'board-a' }),
        publish: () => {},
        isCurrent: () => true,
      });

      return { cache, key };
    }

    it('keeps an aged entry hydratable so a view paints without waiting on the network', async () => {
      const { cache, key } = await cacheWithBoard();

      expect(cache.markStaleTags(['board:board-a'], principal, workspaceId)).toBe(true);

      const published: unknown[] = [];
      const hydrated = await cache.hydrate({
        key,
        payloadSchema,
        publish: (payload) => published.push(payload),
        isCurrent: () => true,
      });

      expect(hydrated).toEqual({ id: 'board-a' });
      expect(published).toEqual([{ id: 'board-a' }]);
    });

    it('refuses an aged entry as fresh so it can never skip the revalidation', async () => {
      const { cache, key } = await cacheWithBoard();

      const request = {
        key,
        payloadSchema,
        freshForMs: 120_000,
        publish: () => {},
        isCurrent: () => true,
      };

      expect(cache.readFresh(request)).toEqual({ id: 'board-a' });

      cache.markStaleTags(['board:board-a'], principal, workspaceId);

      expect(cache.readFresh(request)).toBeNull();
    });

    it('leaves an entry carrying none of the aged tags fresh', async () => {
      const { cache, key } = await cacheWithBoard();

      cache.markStaleTags(['board:board-b'], principal, workspaceId);

      expect(
        cache.readFresh({
          key,
          payloadSchema,
          freshForMs: 120_000,
          publish: () => {},
          isCurrent: () => true,
        }),
      ).toEqual({ id: 'board-a' });
    });

    it('blocks rather than ages anything when the principal is unknown', async () => {
      const { cache, key } = await cacheWithBoard();

      expect(cache.markStaleTags(['board:board-a'])).toBe(false);
      expect(cache.isAvailable()).toBe(false);
      expect(
        await cache.hydrate({ key, payloadSchema, publish: () => {}, isCurrent: () => true }),
      ).toBeNull();
    });
  });

  describe('a persistent store that cannot be reached', () => {
    const payloadSchema = z.object({ id: z.string() });

    /**
     * A store shaped like one whose backing database never opens: every
     * operation reports failure and it admits it is unreachable. What a webview
     * that denies IndexedDB looks like from the cache's side.
     */
    function unreachableStore(): ResourceCacheStore {
      return {
        get: async () => null,
        putMany: async () => false,
        deleteMany: async () => false,
        deleteScope: async () => false,
        clear: async () => false,
        isAvailable: () => false,
      };
    }

    function boardRequest(key: string) {
      return {
        key,
        payloadSchema,
        tags: ['board:board-a'],
        freshForMs: 120_000,
        retentionForMs: 600_000,
        load: async () => ({ id: 'board-a' }),
        publish: () => {},
        isCurrent: () => true,
      };
    }

    function boardKey(): string {
      const key = buildCacheKey({
        principal,
        workspaceId,
        resourceKind: 'task-board',
        resourceId: 'board-a',
      });
      if (key === null) throw new Error('expected a canonical cache key');
      return key;
    }

    it('degrades to caching in memory instead of caching nothing', async () => {
      const cache = new ResourceCache({ store: unreachableStore(), policy: DEFAULT_CACHE_POLICY });
      cache.allow();
      const key = boardKey();

      await cache.revalidate(boardRequest(key));

      // The whole point: a revisit still answers from memory, so the view paints
      // without a spinner even though nothing could be written to disk.
      expect(
        cache.readFresh({
          key,
          payloadSchema,
          freshForMs: 120_000,
          publish: () => {},
          isCurrent: () => true,
        }),
      ).toEqual({ id: 'board-a' });
    });

    it('stays usable after a purge it could not carry out on disk', async () => {
      const cache = new ResourceCache({ store: unreachableStore(), policy: DEFAULT_CACHE_POLICY });
      cache.allow();
      const key = boardKey();
      await cache.revalidate(boardRequest(key));

      expect(await cache.purgeTags(['board:board-a'], principal, workspaceId)).toBe(true);
      expect(cache.isAvailable()).toBe(true);

      // The purge still took effect where it could: memory no longer holds it.
      expect(
        cache.readFresh({
          key,
          payloadSchema,
          freshForMs: 120_000,
          publish: () => {},
          isCurrent: () => true,
        }),
      ).toBeNull();

      // And the cache keeps working afterwards rather than being dead for good.
      await cache.revalidate(boardRequest(key));
      expect(
        cache.readFresh({
          key,
          payloadSchema,
          freshForMs: 120_000,
          publish: () => {},
          isCurrent: () => true,
        }),
      ).toEqual({ id: 'board-a' });
    });

    it('still treats a reachable store that rejects the write as a hazard', async () => {
      const cache = new ResourceCache({
        store: { ...unreachableStore(), isAvailable: () => true },
        policy: DEFAULT_CACHE_POLICY,
      });
      cache.allow();
      const key = boardKey();

      await cache.revalidate(boardRequest(key));

      expect(
        cache.readFresh({
          key,
          payloadSchema,
          freshForMs: 120_000,
          publish: () => {},
          isCurrent: () => true,
        }),
      ).toBeNull();
    });

    it('blocks when a reachable store fails to purge', async () => {
      const cache = new ResourceCache({
        store: { ...unreachableStore(), isAvailable: () => true },
        policy: DEFAULT_CACHE_POLICY,
      });
      cache.allow();

      expect(await cache.purgeTags(['board:board-a'], principal, workspaceId)).toBe(false);
      expect(cache.isAvailable()).toBe(false);
    });
  });
});
