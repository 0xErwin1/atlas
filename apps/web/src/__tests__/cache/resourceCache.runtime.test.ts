import { describe, expect, it, vi } from 'vitest';
import { z } from 'zod';
import {
  allowResourceCache,
  blockAndPurgeResourceCache,
  configureResourceCacheForTest,
  hardRefreshResourceCache,
  hydrateAndRevalidateResource,
  invalidateLiveResourceCache,
  purgeResourceCache,
  runHardRefresh,
  setResourceCachePrincipal,
} from '@/cache/cacheRuntime';
import {
  CACHE_CADENCE,
  type CacheEnvelope,
  ResourceCache,
  type ResourceCacheRequest,
  type ResourceCacheStore,
} from '@/cache/resourceCache';

const payloadSchema = z.object({ title: z.string() });

function createEntry(key: string, title: string, now: number) {
  return {
    schema: 1 as const,
    key,
    payloadVersion: 1,
    storedAt: now,
    validatedAt: now,
    lastAccessedAt: now,
    retentionExpiresAt: now + 60_000,
    bytes: 32,
    stale: false,
    tags: ['workspace:workspace-a'],
    payload: { title },
  };
}

describe('ResourceCache runtime', () => {
  it('uses the distinct catalog, primary, and secondary cadence policies', () => {
    expect(CACHE_CADENCE.catalog).toEqual({ freshForMs: 30_000, activeForMs: 60_000 });
    expect(CACHE_CADENCE.primary).toEqual({ freshForMs: 120_000, activeForMs: 300_000 });
    expect(CACHE_CADENCE.secondary).toEqual({ freshForMs: 60_000, activeForMs: 120_000 });
  });

  it('uses a learned broker alias to purge only that workspace for malformed recovery', async () => {
    const principal = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const workspaceId = '019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const purgeWorkspace = vi.fn().mockResolvedValue(true);
    const purge = vi.fn().mockResolvedValue(true);
    configureResourceCacheForTest({ allow: vi.fn(), block: vi.fn(), purgeWorkspace, purge });
    setResourceCachePrincipal(principal);

    await invalidateLiveResourceCache(
      {
        id: 'event-1',
        event_type: 'unknown.event',
        version: 1,
        source: 'test',
        workspace_id: workspaceId,
        occurred_at: '2026-01-01T00:00:00Z',
        actor: { type: 'user', id: 'user-1' },
        data: {},
      },
      'acme',
    );
    await invalidateLiveResourceCache(undefined, 'acme');

    expect(purgeWorkspace).toHaveBeenCalledWith(workspaceId, principal);
    expect(purge).not.toHaveBeenCalled();
  });

  it('does not let a wrong-workspace unknown envelope poison the alias used by later broker recovery', async () => {
    const principal = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const brokerWorkspaceId = '019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const wrongWorkspaceId = '019ef171-bbcf-7b90-9be6-5dbb382afd09';
    const purgeWorkspace = vi.fn().mockResolvedValue(true);
    const purgeTags = vi.fn().mockResolvedValue(true);
    const purge = vi.fn().mockResolvedValue(true);
    configureResourceCacheForTest({ allow: vi.fn(), block: vi.fn(), purgeTags, purgeWorkspace, purge });
    setResourceCachePrincipal(principal);

    await invalidateLiveResourceCache(
      {
        id: 'event-valid',
        event_type: 'task.created',
        version: 1,
        source: 'test',
        workspace_id: brokerWorkspaceId,
        occurred_at: '2026-01-01T00:00:00Z',
        actor: { type: 'user', id: 'user-1' },
        data: { task_id: '019ef171-bbcf-7b90-9be6-5dbb382afd10' },
      },
      'acme',
    );
    purgeWorkspace.mockClear();

    await invalidateLiveResourceCache(
      {
        id: 'event-wrong',
        event_type: 'unknown.event',
        version: 1,
        source: 'test',
        workspace_id: wrongWorkspaceId,
        occurred_at: '2026-01-01T00:00:00Z',
        actor: { type: 'user', id: 'user-1' },
        data: {},
      },
      'acme',
    );
    purgeWorkspace.mockClear();

    await expect(invalidateLiveResourceCache(undefined, 'acme')).resolves.toBe(true);

    expect(purgeWorkspace).toHaveBeenCalledWith(brokerWorkspaceId, principal);
    expect(purgeWorkspace).not.toHaveBeenCalledWith(wrongWorkspaceId, principal);
    expect(purge).not.toHaveBeenCalled();
  });

  it('requires the current principal to re-register a same-slug broker alias before recovery', async () => {
    const principalA = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const principalB = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd09';
    const workspaceA = '019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const workspaceB = '019ef171-bbcf-7b90-9be6-5dbb382afd09';
    const purgeWorkspace = vi.fn().mockResolvedValue(true);
    const purge = vi.fn().mockResolvedValue(true);
    configureResourceCacheForTest({ allow: vi.fn(), block: vi.fn(), purgeWorkspace, purge });
    setResourceCachePrincipal(principalA);

    await invalidateLiveResourceCache(
      {
        id: 'event-a',
        event_type: 'unknown.event',
        version: 1,
        source: 'test',
        workspace_id: workspaceA,
        occurred_at: '2026-01-01T00:00:00Z',
        actor: { type: 'user', id: 'user-a' },
        data: {},
      },
      'acme',
    );
    purgeWorkspace.mockClear();
    purge.mockClear();

    setResourceCachePrincipal(principalB);

    await expect(invalidateLiveResourceCache(undefined, 'acme')).resolves.toBe(false);
    expect(purgeWorkspace).not.toHaveBeenCalled();
    expect(purge).not.toHaveBeenCalled();

    await invalidateLiveResourceCache(
      {
        id: 'event-b',
        event_type: 'unknown.event',
        version: 1,
        source: 'test',
        workspace_id: workspaceB,
        occurred_at: '2026-01-01T00:00:00Z',
        actor: { type: 'user', id: 'user-b' },
        data: {},
      },
      'acme',
    );
    await expect(invalidateLiveResourceCache(undefined, 'acme')).resolves.toBe(true);

    expect(purgeWorkspace).toHaveBeenLastCalledWith(workspaceB, principalB);
    expect(purgeWorkspace).not.toHaveBeenCalledWith(workspaceA, principalB);
  });

  it('fails closed for an unknown broker alias without deleting another workspace or principal', async () => {
    const purgeWorkspace = vi.fn().mockResolvedValue(true);
    const purge = vi.fn().mockResolvedValue(true);
    configureResourceCacheForTest({ allow: vi.fn(), block: vi.fn(), purgeWorkspace, purge });
    setResourceCachePrincipal('user:019ef171-bbcf-7b90-9be6-5dbb382afd09');

    await expect(invalidateLiveResourceCache(undefined, 'unknown')).resolves.toBe(false);

    expect(purgeWorkspace).not.toHaveBeenCalled();
    expect(purge).not.toHaveBeenCalled();
  });

  it('keeps a failed global purge closed for the same principal while allowing a different principal namespace', async () => {
    const principalA = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const principalB = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd09';
    const store = {
      get: vi.fn().mockResolvedValue(createEntry('key-b', 'Principal B', Date.now())),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(false),
    };
    const cache = new ResourceCache({ store });
    configureResourceCacheForTest(cache);
    setResourceCachePrincipal(principalA);
    allowResourceCache();

    await expect(blockAndPurgeResourceCache()).resolves.toBe(false);

    allowResourceCache();
    await expect(
      cache.hydrate({
        key: 'key-a',
        payloadSchema,
        publish: vi.fn(),
        isCurrent: () => true,
      }),
    ).resolves.toBeNull();

    setResourceCachePrincipal(principalB);
    allowResourceCache();
    await expect(
      cache.hydrate({
        key: 'key-b',
        payloadSchema,
        publish: vi.fn(),
        isCurrent: () => true,
      }),
    ).resolves.toEqual({ title: 'Principal B' });
  });

  it('hydrates under a valid lease and revalidates an active catalog key after its freshness TTL', async () => {
    let now = 1_000;
    let scheduled: (() => void) | undefined;
    const store = {
      get: vi.fn().mockResolvedValue(createEntry('key-a', 'Cached', now)),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({
      store,
      clock: { now: () => now },
      random: { next: () => 0.5 },
      timer: {
        clear: vi.fn(),
        schedule: (_delay, callback) => {
          scheduled = callback;
          return 1;
        },
      },
    });
    const publish = vi.fn();
    const load = vi.fn().mockResolvedValue({ title: 'Fresh' });
    const request = {
      key: 'key-a',
      payloadSchema,
      tags: ['workspace:workspace-a'],
      freshForMs: 30_000,
      retentionForMs: 60_000,
      load,
      publish,
      isCurrent: () => true,
    };

    cache.allow();
    await cache.hydrate(request);
    cache.activate(request);
    now += 30_000;
    scheduled?.();
    await vi.waitFor(() => expect(load).toHaveBeenCalledOnce());
    await vi.waitFor(() => expect(publish).toHaveBeenLastCalledWith({ title: 'Fresh' }));

    expect(publish).toHaveBeenNthCalledWith(1, { title: 'Cached' });
  });

  it('starts revalidation during hydration and prevents a late cache result from replacing fresh data', async () => {
    let resolveGet: ((entry: CacheEnvelope<{ title: string }> | null) => void) | undefined;
    let resolveLoad: ((payload: { title: string }) => void) | undefined;
    const store: ResourceCacheStore = {
      get: vi.fn(
        <T>(_key: string, schema: z.ZodType<T>) =>
          new Promise<CacheEnvelope<T> | null>((resolve) => {
            resolveGet = (entry) => {
              resolve(entry === null ? null : { ...entry, payload: schema.parse(entry.payload) });
            };
          }),
      ) as ResourceCacheStore['get'],
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store });
    const publish = vi.fn();
    const load = vi.fn(
      () =>
        new Promise<{ title: string }>((resolve) => {
          resolveLoad = resolve;
        }),
    );
    const request = {
      key: 'key-a',
      payloadSchema,
      tags: ['workspace:workspace-a'],
      freshForMs: 30_000,
      retentionForMs: 60_000,
      load,
      publish,
      isCurrent: () => true,
    };

    cache.allow();
    const { hydration, revalidation } = cache.hydrateAndRevalidate(request);

    expect(load).toHaveBeenCalledOnce();

    resolveLoad?.({ title: 'Fresh' });
    await revalidation;
    resolveGet?.(createEntry('key-a', 'Cached', Date.now()));
    await hydration;

    expect(publish).toHaveBeenCalledOnce();
    expect(publish).toHaveBeenCalledWith({ title: 'Fresh' });
  });

  it('keeps cached-first publication when hydration wins the revalidation race', async () => {
    let resolveLoad: ((payload: { title: string }) => void) | undefined;
    const store = {
      get: vi.fn().mockResolvedValue(createEntry('key-a', 'Cached', Date.now())),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store });
    const publish = vi.fn();
    const request = {
      key: 'key-a',
      payloadSchema,
      tags: ['workspace:workspace-a'],
      freshForMs: 30_000,
      retentionForMs: 60_000,
      load: () =>
        new Promise<{ title: string }>((resolve) => {
          resolveLoad = resolve;
        }),
      publish,
      isCurrent: () => true,
    };

    cache.allow();
    const { hydration, revalidation } = cache.hydrateAndRevalidate(request);
    await hydration;
    resolveLoad?.({ title: 'Fresh' });
    await revalidation;

    expect(publish).toHaveBeenNthCalledWith(1, { title: 'Cached' });
    expect(publish).toHaveBeenNthCalledWith(2, { title: 'Fresh' });
  });

  it('settles authoritative success without waiting for hung hydration', async () => {
    const store: ResourceCacheStore = {
      get: vi.fn(() => new Promise(() => undefined)) as ResourceCacheStore['get'],
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store });
    const publish = vi.fn();
    const request = {
      key: 'key-a',
      payloadSchema,
      tags: ['workspace:workspace-a'],
      freshForMs: 30_000,
      retentionForMs: 60_000,
      load: vi.fn().mockResolvedValue({ title: 'Fresh' }),
      publish,
      isCurrent: () => true,
    };

    cache.allow();
    const operation = cache.hydrateAndRevalidate(request);

    await expect(operation.completion).resolves.toMatchObject({
      payload: { title: 'Fresh' },
      published: true,
    });
    expect(publish).toHaveBeenCalledOnce();
  });

  it('settles authoritative denial and suppresses cache data that hydrates later', async () => {
    let resolveGet: ((entry: CacheEnvelope<{ title: string }>) => void) | undefined;
    const store: ResourceCacheStore = {
      get: vi.fn(
        <T>(_key: string, schema: z.ZodType<T>) =>
          new Promise<CacheEnvelope<T> | null>((resolve) => {
            resolveGet = (entry) => resolve({ ...entry, payload: schema.parse(entry.payload) });
          }),
      ) as ResourceCacheStore['get'],
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store });
    const publish = vi.fn();
    const denial = Object.assign(new Error('Denied'), { status: 403 });
    const request = {
      key: 'key-a',
      payloadSchema,
      tags: ['workspace:workspace-a'],
      freshForMs: 30_000,
      retentionForMs: 60_000,
      load: vi.fn().mockRejectedValue(denial),
      publish,
      isCurrent: () => true,
    };

    cache.allow();
    const operation = cache.hydrateAndRevalidate(request);
    await expect(operation.completion).rejects.toBe(denial);

    resolveGet?.(createEntry('key-a', 'Cached', Date.now()));
    await operation.hydration;

    expect(publish).not.toHaveBeenCalled();
  });

  it('does not activate a request that stops being current before authoritative completion', async () => {
    let current = true;
    let resolveLoad: ((payload: { title: string }) => void) | undefined;
    const store = {
      get: vi.fn().mockResolvedValue(null),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store });
    const activate = vi.spyOn(cache, 'activate');
    const request = {
      key: 'key-a',
      payloadSchema,
      tags: ['workspace:workspace-a'],
      freshForMs: 30_000,
      retentionForMs: 60_000,
      load: () =>
        new Promise<{ title: string }>((resolve) => {
          resolveLoad = resolve;
        }),
      publish: vi.fn(),
      isCurrent: () => current,
    };

    cache.allow();
    const operation = cache.hydrateAndRevalidate(request);
    current = false;
    resolveLoad?.({ title: 'Fresh' });
    await operation.completion;

    expect(activate).not.toHaveBeenCalled();
  });

  it('does not persist or activate an old-principal request after the runtime epoch changes', async () => {
    let resolveLoad: ((payload: { title: string }) => void) | undefined;
    const oldPrincipal = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const nextPrincipal = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd09';
    const workspaceId = '019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const store = {
      get: vi.fn().mockResolvedValue(null),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store });
    const activate = vi.spyOn(cache, 'activate');
    configureResourceCacheForTest(cache);
    setResourceCachePrincipal(oldPrincipal);
    allowResourceCache();
    const request = {
      key: `v1|p=${oldPrincipal}|w=${workspaceId}|k=note-body|r=note-a|q={}`,
      payloadSchema,
      tags: ['document:note-a'],
      freshForMs: 30_000,
      retentionForMs: 60_000,
      load: () =>
        new Promise<{ title: string }>((resolve) => {
          resolveLoad = resolve;
        }),
      publish: vi.fn(),
      isCurrent: () => true,
    };

    const operation = hydrateAndRevalidateResource(request);
    setResourceCachePrincipal(nextPrincipal);
    allowResourceCache();
    resolveLoad?.({ title: 'Late' });
    await expect(operation.completion).resolves.toMatchObject({ published: false });

    expect(store.putMany).not.toHaveBeenCalled();
    expect(activate).not.toHaveBeenCalled();
    expect(request.publish).not.toHaveBeenCalled();
  });

  it('purges without waiting for hung HTTP and delivers its late response without re-persisting it', async () => {
    let resolveLoad: ((payload: { title: string }) => void) | undefined;
    const store = {
      get: vi.fn().mockResolvedValue(null),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store });
    const publish = vi.fn();
    const request = {
      key: 'key-a',
      payloadSchema,
      tags: ['workspace:workspace-a'],
      freshForMs: 30_000,
      retentionForMs: 60_000,
      load: () =>
        new Promise<{ title: string }>((resolve) => {
          resolveLoad = resolve;
        }),
      publish,
      isCurrent: () => true,
    };

    cache.allow();
    const operation = cache.hydrateAndRevalidate(request);
    const purge = cache.purge();

    await vi.waitFor(() => expect(store.clear).toHaveBeenCalledOnce());
    await expect(purge).resolves.toBe(true);

    resolveLoad?.({ title: 'Late' });
    // The fetch succeeded over HTTP, so the still-current caller receives the
    // payload instead of a payloadless "success" — but the fenced context is
    // never re-persisted (no putMany into the purged scope).
    await expect(operation.completion).resolves.toEqual({ published: true, payload: { title: 'Late' } });
    expect(publish).toHaveBeenCalledWith({ title: 'Late' });
    expect(store.putMany).not.toHaveBeenCalled();
  });

  it('purges only the requested workspace and prevents stale work from restoring it', async () => {
    let resolveLoad: ((payload: { title: string }) => void) | undefined;
    const store = {
      get: vi.fn(),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store });
    const request = {
      key: 'v1|p=user:019ef171-bbcf-7b90-9be6-5dbb382afd08|w=019ef171-bbcf-7b90-9be6-5dbb382afd08|k=note-body|r=note-a|q={}',
      payloadSchema,
      tags: ['workspace:019ef171-bbcf-7b90-9be6-5dbb382afd08', 'document:note-a'],
      freshForMs: 30_000,
      retentionForMs: 60_000,
      load: () =>
        new Promise((resolve) => {
          resolveLoad = resolve;
        }),
      publish: vi.fn(),
      isCurrent: () => true,
    };

    const revalidation = cache.revalidate(request);
    const purge = cache.purgeWorkspace('019ef171-bbcf-7b90-9be6-5dbb382afd08');
    resolveLoad?.({ title: 'Stale' });
    await Promise.all([revalidation, purge]);

    expect(store.clear).not.toHaveBeenCalled();
    expect(store.putMany).not.toHaveBeenCalled();
  });

  it('preserves another principal hot entry while invalidating the requested principal workspace', async () => {
    const workspaceId = '019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const principalA = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const principalB = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd09';
    const now = Date.now();
    const store = {
      get: vi
        .fn()
        .mockResolvedValueOnce(
          createEntry(`v1|p=${principalA}|w=${workspaceId}|k=note-body|r=a|q={}`, 'A', now),
        )
        .mockResolvedValueOnce(
          createEntry(`v1|p=${principalB}|w=${workspaceId}|k=note-body|r=b|q={}`, 'B', now),
        ),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      deleteScope: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store });
    cache.allow();

    await cache.hydrate({
      key: `v1|p=${principalA}|w=${workspaceId}|k=note-body|r=a|q={}`,
      payloadSchema,
      publish: vi.fn(),
      isCurrent: () => true,
    });
    await cache.hydrate({
      key: `v1|p=${principalB}|w=${workspaceId}|k=note-body|r=b|q={}`,
      payloadSchema,
      publish: vi.fn(),
      isCurrent: () => true,
    });

    await expect(cache.purgeWorkspace(workspaceId, principalA)).resolves.toBe(true);
    await expect(
      cache.hydrate({
        key: `v1|p=${principalB}|w=${workspaceId}|k=note-body|r=b|q={}`,
        payloadSchema,
        publish: vi.fn(),
        isCurrent: () => true,
      }),
    ).resolves.toEqual({ title: 'B' });
  });

  it('completes the workspace purge before a hard refresh reloads the route', async () => {
    const events: string[] = [];
    setResourceCachePrincipal('user:019ef171-bbcf-7b90-9be6-5dbb382afd08');
    configureResourceCacheForTest({
      purgeWorkspace: vi.fn().mockImplementation(async () => {
        events.push('purge');
        return true;
      }),
    });

    await runHardRefresh('019ef171-bbcf-7b90-9be6-5dbb382afd08', async () => {
      events.push('reload');
    });

    expect(events).toEqual(['purge', 'reload']);
  });

  it('passes the current principal through a hard refresh so cold scoped entries can be deleted', async () => {
    const purgeWorkspace = vi.fn().mockResolvedValue(true);
    configureResourceCacheForTest({ purgeWorkspace });
    setResourceCachePrincipal('user:019ef171-bbcf-7b90-9be6-5dbb382afd08');

    await expect(hardRefreshResourceCache('019ef171-bbcf-7b90-9be6-5dbb382afd08')).resolves.toBe(true);

    expect(purgeWorkspace).toHaveBeenCalledWith(
      '019ef171-bbcf-7b90-9be6-5dbb382afd08',
      'user:019ef171-bbcf-7b90-9be6-5dbb382afd08',
    );
  });

  it('fails closed when the current principal is unresolved or not canonical', async () => {
    const purgeWorkspace = vi.fn().mockResolvedValue(true);
    const canonicalWorkspaceId = '019ef171-bbcf-7b90-9be6-5dbb382afd08';
    configureResourceCacheForTest({ purgeWorkspace });

    setResourceCachePrincipal(undefined);
    await expect(hardRefreshResourceCache(canonicalWorkspaceId)).resolves.toBe(false);

    setResourceCachePrincipal('user:not-a-uuid');
    await expect(hardRefreshResourceCache(canonicalWorkspaceId)).resolves.toBe(false);

    setResourceCachePrincipal('user:019ef171-bbcf-7b90-9be6-5dbb382afd08');
    await expect(hardRefreshResourceCache('workspace-slug')).resolves.toBe(false);

    expect(purgeWorkspace).not.toHaveBeenCalled();
  });

  it('fails closed before scoped deletion when a resource cache has no principal', async () => {
    const store = {
      get: vi.fn(),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      deleteScope: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store });

    await expect(cache.purgeWorkspace('019ef171-bbcf-7b90-9be6-5dbb382afd08')).resolves.toBe(false);

    expect(store.deleteScope).not.toHaveBeenCalled();
  });

  it('settles a pending write before deleting its principal workspace tag scope', async () => {
    let resolvePut: ((result: boolean) => void) | undefined;
    let resolveLoad: ((payload: { title: string }) => void) | undefined;
    const principal = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const workspaceId = '019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const store = {
      get: vi.fn(),
      putMany: vi.fn(
        () =>
          new Promise<boolean>((resolve) => {
            resolvePut = resolve;
          }),
      ),
      deleteMany: vi.fn().mockResolvedValue(true),
      deleteScope: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store });
    const publish = vi.fn();
    const key = `v1|p=${principal}|w=${workspaceId}|k=note-body|r=note-a|q={}`;
    const revalidation = cache.revalidate({
      key,
      payloadSchema,
      tags: [`workspace:${workspaceId}`, 'document:note-a'],
      freshForMs: 30_000,
      retentionForMs: 60_000,
      load: () =>
        new Promise((resolve) => {
          resolveLoad = resolve;
        }),
      publish,
      isCurrent: () => true,
    });

    resolveLoad?.({ title: 'Fresh' });
    await vi.waitFor(() => expect(store.putMany).toHaveBeenCalledOnce());
    const purge = cache.purgeTags(['document:note-a'], principal, workspaceId);
    resolvePut?.(true);
    await Promise.all([revalidation, purge]);

    expect(store.deleteMany).toHaveBeenCalledWith([key]);
    expect(store.deleteScope).toHaveBeenCalledWith({
      principal,
      workspaceId,
      tagsAny: ['document:note-a'],
    });
    expect(publish).not.toHaveBeenCalled();
  });

  it('returns a failed-persistence payload to a current caller after its scoped purge without re-caching it', async () => {
    let resolvePut: ((result: boolean) => void) | undefined;
    const principal = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const workspaceId = '019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const key = `v1|p=${principal}|w=${workspaceId}|k=note-body|r=note-a|q={}`;
    const store = {
      get: vi.fn().mockResolvedValue(null),
      putMany: vi.fn(
        () =>
          new Promise<boolean>((resolve) => {
            resolvePut = resolve;
          }),
      ),
      deleteMany: vi.fn().mockResolvedValue(true),
      deleteScope: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store });
    const publish = vi.fn();

    cache.allow();
    const operation = cache.hydrateAndRevalidate({
      key,
      payloadSchema,
      tags: [`workspace:${workspaceId}`, 'document:note-a'],
      freshForMs: 30_000,
      retentionForMs: 60_000,
      load: vi.fn().mockResolvedValue({ title: 'Pre-purge' }),
      publish,
      isCurrent: () => true,
    });

    await vi.waitFor(() => expect(store.putMany).toHaveBeenCalledOnce());
    await expect(cache.purgeTags(['document:note-a'], principal, workspaceId)).resolves.toBe(true);
    resolvePut?.(false);

    // Persistence failed and its scoped purge fenced the cache write, but the
    // payload was fetched successfully, so the current caller still receives it.
    await expect(operation.completion).resolves.toEqual({
      published: true,
      payload: { title: 'Pre-purge' },
    });
    expect(publish).toHaveBeenCalledWith({ title: 'Pre-purge' });
  });

  it('never hydrates or revalidates data that resolves after its scoped purge begins', async () => {
    let resolveGet: (() => void) | undefined;
    let resolveLoad: ((payload: { title: string }) => void) | undefined;
    let resolveDelete: ((result: boolean) => void) | undefined;
    let pendingGet = true;
    const principal = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const workspaceId = '019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const key = `v1|p=${principal}|w=${workspaceId}|k=note-body|r=note-a|q={}`;
    const store = {
      get: vi.fn(<T>(_key: string, entrySchema: z.ZodType<T>): Promise<CacheEnvelope<T> | null> => {
        if (!pendingGet) return Promise.resolve(null);
        return new Promise<CacheEnvelope<T>>((resolve) => {
          resolveGet = () => {
            const entry = createEntry(key, 'Cached', Date.now());
            resolve({ ...entry, payload: entrySchema.parse(entry.payload) });
          };
        });
      }) as ResourceCacheStore['get'],
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      deleteScope: vi.fn(
        () =>
          new Promise<boolean>((resolve) => {
            resolveDelete = resolve;
          }),
      ),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store });
    const hydratedPublish = vi.fn();
    const revalidatedPublish = vi.fn();

    cache.allow();
    const hydration = cache.hydrate({ key, payloadSchema, publish: hydratedPublish, isCurrent: () => true });
    const revalidation = cache.revalidate({
      key,
      payloadSchema,
      tags: [`workspace:${workspaceId}`, 'document:note-a'],
      freshForMs: 30_000,
      retentionForMs: 60_000,
      load: () =>
        new Promise((resolve) => {
          resolveLoad = resolve;
        }),
      publish: revalidatedPublish,
      isCurrent: () => true,
    });
    await vi.waitFor(() => expect(store.get).toHaveBeenCalledOnce());
    await vi.waitFor(() => expect(resolveLoad).toBeTypeOf('function'));
    const purge = cache.purgeTags(['document:note-a'], principal, workspaceId);

    pendingGet = false;
    resolveGet?.();
    resolveLoad?.({ title: 'Fresh' });
    await vi.waitFor(() => expect(store.deleteScope).toHaveBeenCalledOnce());

    expect(hydratedPublish).not.toHaveBeenCalled();
    expect(revalidatedPublish).not.toHaveBeenCalled();
    expect(store.putMany).not.toHaveBeenCalled();

    resolveDelete?.(true);
    // Nothing is published or cached into the purged scope, but the fetched
    // payload is still handed back on the revalidation result rather than
    // discarded — a caller must never see a payloadless success.
    await expect(Promise.all([hydration, revalidation, purge])).resolves.toEqual([
      null,
      { published: false, payload: { title: 'Fresh' } },
      true,
    ]);

    await expect(
      cache.hydrate({ key, payloadSchema, publish: vi.fn(), isCurrent: () => true }),
    ).resolves.toBeNull();
  });

  it('purges matching hot tags only within the requested principal and workspace', async () => {
    const principal = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const otherPrincipal = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd09';
    const workspaceId = '019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const otherWorkspaceId = '019ef171-bbcf-7b90-9be6-5dbb382afd09';
    const requestedKey = `v1|p=${principal}|w=${workspaceId}|k=note-body|r=document-a|q={}`;
    const otherWorkspaceKey = `v1|p=${principal}|w=${otherWorkspaceId}|k=note-body|r=document-a|q={}`;
    const otherPrincipalKey = `v1|p=${otherPrincipal}|w=${workspaceId}|k=note-body|r=document-a|q={}`;
    const entries = new Map<string, ReturnType<typeof createEntry>>([
      [
        requestedKey,
        { ...createEntry(requestedKey, 'Requested', Date.now()), tags: ['document:document-a'] },
      ],
      [
        otherWorkspaceKey,
        { ...createEntry(otherWorkspaceKey, 'Other workspace', Date.now()), tags: ['document:document-a'] },
      ],
      [
        otherPrincipalKey,
        { ...createEntry(otherPrincipalKey, 'Other principal', Date.now()), tags: ['document:document-a'] },
      ],
    ]);
    const store = {
      get: vi.fn(<T>(key: string, entrySchema: z.ZodType<T>): Promise<CacheEnvelope<T> | null> => {
        const entry = entries.get(key);
        return Promise.resolve(
          entry === undefined ? null : { ...entry, payload: entrySchema.parse(entry.payload) },
        );
      }) as ResourceCacheStore['get'],
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn(async (keys: readonly string[]) => {
        for (const key of keys) entries.delete(key);
        return true;
      }),
      deleteScope: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store });

    cache.allow();
    for (const key of entries.keys()) {
      await cache.hydrate({ key, payloadSchema, publish: vi.fn(), isCurrent: () => true });
    }

    await expect(cache.purgeTags(['document:document-a'], principal, workspaceId)).resolves.toBe(true);

    expect(store.deleteMany).toHaveBeenCalledWith([requestedKey]);
    expect(
      await cache.hydrate({ key: requestedKey, payloadSchema, publish: vi.fn(), isCurrent: () => true }),
    ).toBeNull();
    expect(
      await cache.hydrate({ key: otherWorkspaceKey, payloadSchema, publish: vi.fn(), isCurrent: () => true }),
    ).toEqual({
      title: 'Other workspace',
    });
    expect(
      await cache.hydrate({ key: otherPrincipalKey, payloadSchema, publish: vi.fn(), isCurrent: () => true }),
    ).toEqual({
      title: 'Other principal',
    });
  });

  it('deletes a cold-only tag scope without requiring a matching hot entry', async () => {
    const principal = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const workspaceId = '019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const store = {
      get: vi.fn(),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      deleteScope: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store });

    await expect(cache.purgeTags(['document:note-a'], principal, workspaceId)).resolves.toBe(true);

    expect(store.deleteMany).not.toHaveBeenCalled();
    expect(store.deleteScope).toHaveBeenCalledWith({
      principal,
      workspaceId,
      tagsAny: ['document:note-a'],
    });
  });

  it('blocks after a failed cold scope deletion and leaves successful scope purges usable', async () => {
    const principal = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const workspaceId = '019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const store = {
      get: vi.fn(),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      deleteScope: vi.fn().mockResolvedValueOnce(false).mockResolvedValueOnce(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store });
    const request = {
      key: `v1|p=${principal}|w=${workspaceId}|k=note-body|r=note-a|q={}`,
      payloadSchema,
      tags: [`workspace:${workspaceId}`, 'document:note-a'],
      freshForMs: 30_000,
      retentionForMs: 60_000,
      load: vi.fn().mockResolvedValue({ title: 'Fresh' }),
      publish: vi.fn(),
      isCurrent: () => true,
    };

    cache.allow();
    await expect(cache.purgeWorkspace(workspaceId, principal)).resolves.toBe(false);
    await cache.revalidate(request);
    expect(request.load).not.toHaveBeenCalled();

    cache.allow();
    await expect(cache.purgeWorkspace(workspaceId, principal)).resolves.toBe(true);
    await cache.revalidate(request);
    expect(request.load).toHaveBeenCalledOnce();
  });

  it.each([
    ['first', 'second'],
    ['second', 'first'],
  ] as const)('keeps all scoped work suspended when %s purge completes before %s', async (firstToFinish, _secondToFinish) => {
    const principal = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const workspaceId = '019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const resolveDeletes: Array<(result: boolean) => void> = [];
    const store = {
      get: vi.fn(),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      deleteScope: vi.fn(
        () =>
          new Promise<boolean>((resolve) => {
            resolveDeletes.push(resolve);
          }),
      ),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store });
    const load = vi.fn().mockResolvedValue({ title: 'Unexpected' });
    const request = {
      key: `v1|p=${principal}|w=${workspaceId}|k=note-body|r=note-a|q={}`,
      payloadSchema,
      tags: [`workspace:${workspaceId}`, 'document:note-a'],
      freshForMs: 30_000,
      retentionForMs: 60_000,
      load,
      publish: vi.fn(),
      isCurrent: () => true,
    };

    const first = cache.purgeTags(['document:note-a'], principal, workspaceId);
    const second = cache.purgeTags(['document:note-b'], principal, workspaceId);
    await vi.waitFor(() => expect(resolveDeletes).toHaveLength(2));

    const purges = [first, second];
    const firstIndex = firstToFinish === 'first' ? 0 : 1;
    const secondIndex = firstIndex === 0 ? 1 : 0;

    resolveDeletes[firstIndex]?.(true);
    await expect(purges[firstIndex]).resolves.toBe(true);
    await cache.revalidate(request);
    expect(load).not.toHaveBeenCalled();

    resolveDeletes[secondIndex]?.(true);
    await expect(purges[secondIndex]).resolves.toBe(true);
    await cache.revalidate(request);
    expect(load).toHaveBeenCalledOnce();
  });

  it('propagates global storage failures, blocks new work, and releases a successful retry', async () => {
    const store = {
      get: vi.fn(),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValueOnce(false).mockResolvedValueOnce(true),
    };
    const cache = new ResourceCache({ store });
    const load = vi.fn().mockResolvedValue({ title: 'Unexpected' });
    const request = {
      key: 'global-failure',
      payloadSchema,
      tags: ['workspace:workspace-a'],
      freshForMs: 30_000,
      retentionForMs: 60_000,
      load,
      publish: vi.fn(),
      isCurrent: () => true,
    };

    cache.allow();
    await expect(cache.purge()).resolves.toBe(false);
    await cache.revalidate(request);

    expect(store.clear).toHaveBeenCalledOnce();
    expect(load).not.toHaveBeenCalled();

    await expect(cache.purge()).resolves.toBe(true);
    await cache.revalidate(request);

    expect(load).toHaveBeenCalledOnce();

    const purge = vi.fn().mockResolvedValue(false);
    const clear = vi.fn().mockResolvedValue(false);
    configureResourceCacheForTest({ purge, clear });

    await expect(blockAndPurgeResourceCache()).resolves.toBe(false);
    await expect(purgeResourceCache()).resolves.toBe(false);
    expect(purge).toHaveBeenCalledOnce();
    expect(clear).toHaveBeenCalledOnce();
  });

  it('does not release an overlapping global purge until every clear succeeds', async () => {
    const resolveClears: Array<(result: boolean) => void> = [];
    const store = {
      get: vi.fn(),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      clear: vi.fn(
        () =>
          new Promise<boolean>((resolve) => {
            resolveClears.push(resolve);
          }),
      ),
    };
    const cache = new ResourceCache({ store });
    const load = vi.fn().mockResolvedValue({ title: 'Unexpected' });
    const request = {
      key: 'global-overlap',
      payloadSchema,
      tags: ['workspace:workspace-a'],
      freshForMs: 30_000,
      retentionForMs: 60_000,
      load,
      publish: vi.fn(),
      isCurrent: () => true,
    };

    cache.allow();
    const first = cache.purge();
    const second = cache.purge();
    await vi.waitFor(() => expect(resolveClears).toHaveLength(2));

    resolveClears[1]?.(true);
    await expect(second).resolves.toBe(true);
    await cache.revalidate(request);
    expect(load).not.toHaveBeenCalled();

    resolveClears[0]?.(false);
    await expect(first).resolves.toBe(false);
    await cache.revalidate(request);
    expect(load).not.toHaveBeenCalled();
  });

  it('hydrates an exact cold entry but neither persists nor publishes a non-current revalidation', async () => {
    const now = 1_000;
    const store = {
      get: vi.fn().mockResolvedValue(createEntry('key-a', 'Cached', now)),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store, clock: { now: () => now } });
    const publish = vi.fn();
    let resolveFetch: ((value: { title: string }) => void) | undefined;

    cache.allow();
    await cache.hydrate({ key: 'key-a', payloadSchema, publish, isCurrent: () => true });
    const revalidation = cache.revalidate({
      key: 'key-a',
      payloadSchema,
      tags: ['workspace:workspace-a'],
      freshForMs: 30_000,
      retentionForMs: 60_000,
      load: () =>
        new Promise((resolve) => {
          resolveFetch = resolve;
        }),
      publish,
      isCurrent: () => false,
    });

    resolveFetch?.({ title: 'Fresh' });
    await revalidation;

    expect(publish).toHaveBeenCalledOnce();
    expect(publish).toHaveBeenCalledWith({ title: 'Cached' });
    expect(store.get).toHaveBeenCalledWith('key-a', payloadSchema);
    expect(store.putMany).not.toHaveBeenCalled();
  });

  it('uses bounded jittered retry-not-before times without letting freshness schedule earlier retries', async () => {
    let now = 0;
    const scheduled: Array<{ delay: number; callback: () => void }> = [];
    const store = {
      get: vi.fn(),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({
      store,
      clock: { now: () => now },
      random: { next: () => 0 },
      timer: {
        clear: vi.fn(),
        schedule: (delay, callback) => {
          scheduled.push({ delay, callback });
          return scheduled.length;
        },
      },
    });
    const load = vi.fn().mockRejectedValue(new Error('offline'));
    const request = {
      key: 'catalog-key',
      payloadSchema,
      tags: ['workspace:workspace-a'],
      freshForMs: CACHE_CADENCE.catalog.freshForMs,
      activeForMs: CACHE_CADENCE.catalog.activeForMs,
      retentionForMs: 60_000,
      load,
      publish: vi.fn(),
      isCurrent: () => true,
    };

    cache.activate(request);

    const expectedRetryDelays = [48_000, 96_000, 192_000, 384_000, 720_000];
    for (const [index, expectedDelay] of expectedRetryDelays.entries()) {
      const current = scheduled.at(-1);
      expect(current?.delay).toBe(
        index === 0 ? CACHE_CADENCE.catalog.activeForMs : expectedRetryDelays[index - 1],
      );

      now += current?.delay ?? 0;
      current?.callback();
      await vi.waitFor(() =>
        expect(load).toHaveBeenCalledTimes(expectedRetryDelays.indexOf(expectedDelay) + 1),
      );
      await vi.waitFor(() => expect(scheduled.at(-1)?.delay).toBe(expectedDelay));
    }
  });

  it('replans one shared timer when an earlier active key is added without delaying unchanged later work', () => {
    let now = 0;
    const timer = {
      clear: vi.fn(),
      schedule: vi.fn().mockReturnValue(1),
    };
    const store = {
      get: vi.fn(),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store, clock: { now: () => now }, timer });
    const primary = vi.fn().mockResolvedValue(undefined);
    const catalog = vi.fn().mockResolvedValue(undefined);

    cache.activate('primary', primary, CACHE_CADENCE.primary.activeForMs);
    cache.activate('catalog', catalog, CACHE_CADENCE.catalog.activeForMs);

    expect(timer.clear).toHaveBeenCalledWith(1);
    expect(timer.schedule).toHaveBeenLastCalledWith(CACHE_CADENCE.catalog.activeForMs, expect.any(Function));

    now = 1_000;
    cache.activate('later-primary', primary, CACHE_CADENCE.primary.activeForMs);

    expect(timer.clear).toHaveBeenCalledTimes(1);
    expect(timer.schedule).toHaveBeenLastCalledWith(CACHE_CADENCE.catalog.activeForMs, expect.any(Function));

    const reverseTimer = {
      clear: vi.fn(),
      schedule: vi.fn().mockReturnValue(2),
    };
    const reverseCache = new ResourceCache({ store, clock: { now: () => 0 }, timer: reverseTimer });

    reverseCache.activate('catalog-first', catalog, CACHE_CADENCE.catalog.activeForMs);
    reverseCache.activate('primary-second', primary, CACHE_CADENCE.primary.activeForMs);

    expect(reverseTimer.clear).not.toHaveBeenCalled();
    expect(reverseTimer.schedule).toHaveBeenCalledOnce();
  });

  it('continues scheduling other active keys through multiple intervals while an earlier request remains hung', async () => {
    let now = 0;
    const scheduled: Array<{ delay: number; callback: () => void }> = [];
    const timer = {
      clear: vi.fn(),
      schedule: vi.fn((delay: number, callback: () => void) => {
        scheduled.push({ delay, callback });
        return scheduled.length;
      }),
    };
    const store = {
      get: vi.fn(),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store, clock: { now: () => now }, timer });
    const hung = vi.fn(() => new Promise<void>(() => undefined));
    const later = vi.fn().mockResolvedValue(undefined);

    cache.activate('hung', hung, 100);
    now = 50;
    cache.activate('later', later, 150);

    now = 100;
    scheduled[0]?.callback();
    await vi.waitFor(() => expect(hung).toHaveBeenCalledOnce());
    await vi.waitFor(() => expect(scheduled).toHaveLength(2));
    expect(scheduled[1]?.delay).toBe(100);

    now = 200;
    scheduled[1]?.callback();
    await vi.waitFor(() => expect(later).toHaveBeenCalledOnce());

    await vi.waitFor(() => expect(scheduled.at(-1)?.delay).toBe(100));
    now = 300;
    scheduled.at(-1)?.callback();

    await vi.waitFor(() => expect(scheduled.at(-1)?.delay).toBe(50));
    now = 350;
    scheduled.at(-1)?.callback();
    await vi.waitFor(() => expect(later).toHaveBeenCalledTimes(2));
  });

  it('advances a due fresh key to its freshness boundary without scheduling a zero-delay loop', async () => {
    let now = 0;
    const scheduled: Array<{ delay: number; callback: () => void }> = [];
    const store = {
      get: vi.fn(),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({
      store,
      clock: { now: () => now },
      timer: {
        clear: vi.fn(),
        schedule: (delay, callback) => {
          scheduled.push({ delay, callback });
          return scheduled.length;
        },
      },
    });
    const load = vi.fn().mockResolvedValue({ title: 'Fresh' });
    const request = {
      key: 'fresh-key',
      payloadSchema,
      tags: ['workspace:workspace-a'],
      freshForMs: 500,
      activeForMs: 100,
      retentionForMs: 60_000,
      load,
      publish: vi.fn(),
      isCurrent: () => true,
    };

    await cache.revalidate(request);
    cache.activate(request);
    now = 100;
    scheduled[0]?.callback();
    await vi.waitFor(() => expect(scheduled.at(-1)?.delay).toBe(400));

    expect(load).toHaveBeenCalledOnce();

    now = 500;
    scheduled.at(-1)?.callback();
    await vi.waitFor(() => expect(load).toHaveBeenCalledTimes(2));
    await vi.waitFor(() => expect(scheduled.at(-1)?.delay).toBe(100));
  });

  it('suspends new work until scoped cold deletion finishes and remains blocked when deletion fails', async () => {
    let resolveDelete: ((result: boolean) => void) | undefined;
    const principal = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const workspaceId = '019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const store = {
      get: vi.fn(),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      deleteScope: vi.fn(
        () =>
          new Promise<boolean>((resolve) => {
            resolveDelete = resolve;
          }),
      ),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store });
    const load = vi.fn().mockResolvedValue({ title: 'Unexpected' });
    const request = {
      key: `v1|p=${principal}|w=${workspaceId}|k=note-body|r=note-a|q={}`,
      payloadSchema,
      tags: [`workspace:${workspaceId}`, 'document:note-a'],
      freshForMs: 30_000,
      retentionForMs: 60_000,
      load,
      publish: vi.fn(),
      isCurrent: () => true,
    };

    const purge = cache.purgeTags(['document:note-a'], principal, workspaceId);
    await vi.waitFor(() => expect(store.deleteScope).toHaveBeenCalledOnce());
    cache.activate(request);
    await cache.revalidate(request);
    await cache.retry(request.key);

    expect(load).not.toHaveBeenCalled();
    resolveDelete?.(true);
    await expect(purge).resolves.toBe(true);
    await cache.revalidate(request);
    expect(load).toHaveBeenCalledOnce();

    const failedPurge = cache.purgeTags(['document:note-a'], principal, workspaceId);
    await vi.waitFor(() => expect(store.deleteScope).toHaveBeenCalledTimes(2));
    resolveDelete?.(false);
    await expect(failedPurge).resolves.toBe(false);
    await cache.revalidate(request);
    expect(load).toHaveBeenCalledOnce();
  });

  it('resets retry backoff after a successful active revalidation', async () => {
    let now = 0;
    const scheduled: Array<{ delay: number; callback: () => void }> = [];
    const store = {
      get: vi.fn(),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({
      store,
      clock: { now: () => now },
      random: { next: () => 0.5 },
      timer: {
        clear: vi.fn(),
        schedule: (delay, callback) => {
          scheduled.push({ delay, callback });
          return scheduled.length;
        },
      },
    });
    const load = vi.fn().mockRejectedValueOnce(new Error('offline')).mockResolvedValue({ title: 'Fresh' });
    const request = {
      key: 'primary-key',
      payloadSchema,
      tags: ['workspace:workspace-a'],
      freshForMs: CACHE_CADENCE.primary.freshForMs,
      activeForMs: CACHE_CADENCE.primary.activeForMs,
      retentionForMs: 60_000,
      load,
      publish: vi.fn(),
      isCurrent: () => true,
    };

    cache.activate(request);
    now += CACHE_CADENCE.primary.activeForMs;
    scheduled[0]?.callback();
    await vi.waitFor(() => expect(scheduled.at(-1)?.delay).toBe(60_000));

    now += 60_000;
    scheduled.at(-1)?.callback();
    await vi.waitFor(() => expect(scheduled.at(-1)?.delay).toBe(CACHE_CADENCE.primary.activeForMs));
  });

  it('manually retries an active key without waiting for its TTL or retry-not-before time', async () => {
    const store = {
      get: vi.fn(),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store });
    const publish = vi.fn();
    let resolveLoad: ((payload: { title: string }) => void) | undefined;
    const load = vi.fn(
      () =>
        new Promise<{ title: string }>((resolve) => {
          resolveLoad = resolve;
        }),
    );
    const request = {
      key: 'secondary-key',
      payloadSchema,
      tags: ['workspace:workspace-a'],
      freshForMs: CACHE_CADENCE.secondary.freshForMs,
      activeForMs: CACHE_CADENCE.secondary.activeForMs,
      retentionForMs: 60_000,
      load,
      publish,
      isCurrent: () => true,
    };

    cache.activate(request);
    const retry = cache.retry('secondary-key');
    cache.block();
    resolveLoad?.({ title: 'Fresh' });
    await retry;

    expect(load).toHaveBeenCalledOnce();
    expect(publish).not.toHaveBeenCalled();
    expect(store.putMany).not.toHaveBeenCalled();
  });

  it('cancels scheduler callbacks on block and dispose without reviving registrations after allow', () => {
    const timer = {
      clear: vi.fn(),
      schedule: vi.fn().mockReturnValue(1),
    };
    const store = {
      get: vi.fn(),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store, timer });

    cache.activate('blocked-key', vi.fn().mockResolvedValue(undefined));
    cache.block();
    cache.allow();
    cache.dispose();

    expect(timer.clear).toHaveBeenCalledWith(1);
    expect(timer.schedule).toHaveBeenCalledOnce();
  });

  it('deduplicates concurrent fetches and purges v1 data when disabled', async () => {
    const store = {
      get: vi.fn().mockResolvedValue(createEntry('key-a', 'Cached', 1_000)),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const load = vi.fn().mockResolvedValue({ title: 'Fresh' });
    const cache = new ResourceCache({
      store,
      policy: {
        enabled: false,
        authorizationLeaseMs: 86_400_000,
        hot: { maxEntries: 100 },
        persistent: { maxBytes: 1, maxEntries: 1, maxNoteBodyBytes: 1, maxOtherEntryBytes: 1 },
      },
    });
    const request = {
      key: 'key-a',
      payloadSchema,
      tags: ['workspace:workspace-a'],
      freshForMs: 30_000,
      retentionForMs: 60_000,
      load,
      publish: vi.fn(),
      isCurrent: () => true,
    };

    await Promise.all([cache.revalidate(request), cache.revalidate(request)]);

    expect(load).toHaveBeenCalledOnce();
    expect(store.clear).toHaveBeenCalledOnce();
    expect(store.putMany).not.toHaveBeenCalled();
  });

  it('publishes a deduplicated revalidation through the latest same-key request', async () => {
    let resolveLoad: ((payload: { title: string }) => void) | undefined;
    const store = {
      get: vi.fn().mockResolvedValue(null),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store });
    const load = vi.fn(
      () =>
        new Promise<{ title: string }>((resolve) => {
          resolveLoad = resolve;
        }),
    );
    const firstPublish = vi.fn();
    const latestPublish = vi.fn();
    const request = {
      key: 'key-a',
      payloadSchema,
      tags: ['workspace:workspace-a'],
      freshForMs: 30_000,
      retentionForMs: 60_000,
      load,
    };

    cache.allow();
    const first = cache.revalidate({
      ...request,
      publish: firstPublish,
      isCurrent: () => false,
    });
    const latest = cache.revalidate({
      ...request,
      publish: latestPublish,
      isCurrent: () => true,
    });
    resolveLoad?.({ title: 'Fresh' });

    await expect(Promise.all([first, latest])).resolves.toEqual([
      expect.objectContaining({ published: true }),
      expect.objectContaining({ published: true }),
    ]);
    expect(load).toHaveBeenCalledOnce();
    expect(firstPublish).not.toHaveBeenCalled();
    expect(latestPublish).toHaveBeenCalledOnce();
  });

  it('falls back to the network when durable cache access is unavailable', async () => {
    const store = {
      get: vi.fn().mockRejectedValue(new Error('IndexedDB unavailable')),
      putMany: vi.fn().mockRejectedValue(new Error('IndexedDB unavailable')),
      deleteMany: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store });
    const publish = vi.fn();
    const request = {
      key: 'unavailable-cache-key',
      payloadSchema,
      tags: ['workspace:workspace-a'],
      freshForMs: 30_000,
      retentionForMs: 60_000,
      load: vi.fn().mockResolvedValue({ title: 'Online' }),
      publish,
      isCurrent: () => true,
    };

    cache.allow();

    await expect(cache.hydrate(request)).resolves.toBeNull();
    await expect(cache.revalidate(request)).resolves.toMatchObject({ published: true });

    expect(request.load).toHaveBeenCalledOnce();
    expect(publish).toHaveBeenCalledWith({ title: 'Online' });
  });

  it('does not retain or publish a response when durable persistence resolves false', async () => {
    const store = {
      get: vi.fn().mockResolvedValue(null),
      putMany: vi.fn().mockResolvedValue(false),
      deleteMany: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store });
    const publish = vi.fn();
    const request = {
      key: 'persistence-false-key',
      payloadSchema,
      tags: ['workspace:workspace-a'],
      freshForMs: 30_000,
      retentionForMs: 60_000,
      load: vi.fn().mockResolvedValue({ title: 'Online only' }),
      publish,
      isCurrent: () => true,
    };

    cache.allow();

    await expect(cache.revalidate(request)).resolves.toMatchObject({ published: false });
    await expect(cache.hydrate(request)).resolves.toBeNull();

    expect(store.putMany).toHaveBeenCalledOnce();
    expect(publish).not.toHaveBeenCalled();
  });

  it('stable-merges validated payload-derived tags before persisting and activating a request', async () => {
    const principal = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const workspaceId = '019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const store = {
      get: vi.fn(),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      deleteScope: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store });
    const key = `v1|p=${principal}|w=${workspaceId}|k=task-list|r=workspace-tasks|q={}`;
    const request = {
      key,
      payloadSchema: z.object({ items: z.array(z.object({ id: z.string(), readable_id: z.string() })) }),
      tags: [`workspace:${workspaceId}`, 'workspace-tasks'],
      deriveTags: (payload: { items: Array<{ id: string; readable_id: string }> }) =>
        payload.items.flatMap((task) => [
          `task:${task.readable_id}`,
          `task-uuid:${task.id}`,
          'workspace-tasks',
        ]),
      freshForMs: 30_000,
      retentionForMs: 60_000,
      load: vi.fn().mockResolvedValue({
        items: [{ id: '019ef171-bbcf-7b90-9be6-5dbb382afd09', readable_id: 'ATL-9' }],
      }),
      publish: vi.fn(),
      isCurrent: () => true,
    };

    cache.allow();
    await cache.revalidate(request);
    cache.activate(request);

    expect(store.putMany).toHaveBeenCalledWith([
      expect.objectContaining({
        tags: [
          `workspace:${workspaceId}`,
          'workspace-tasks',
          'task:ATL-9',
          'task-uuid:019ef171-bbcf-7b90-9be6-5dbb382afd09',
        ],
      }),
    ]);

    await expect(
      cache.purgeTags(['task-uuid:019ef171-bbcf-7b90-9be6-5dbb382afd09'], principal, workspaceId),
    ).resolves.toBe(true);
    expect(store.deleteMany).toHaveBeenCalledWith([key]);
  });

  it('rejects an invalid derived tag without persisting or publishing a new payload', async () => {
    const store = {
      get: vi.fn(),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store });
    const publish = vi.fn();

    await expect(
      cache.revalidate({
        key: 'invalid-tag-key',
        payloadSchema,
        tags: ['workspace:workspace-a'],
        deriveTags: () => ['task-uuid:contains whitespace'],
        freshForMs: 30_000,
        retentionForMs: 60_000,
        load: vi.fn().mockResolvedValue({ title: 'Online' }),
        publish,
        isCurrent: () => true,
      }),
    ).rejects.toThrow('Cache tags are invalid.');

    expect(store.putMany).not.toHaveBeenCalled();
    expect(publish).not.toHaveBeenCalled();
  });

  it('accepts a derived tag whose slug contains non-ASCII letters', async () => {
    const store = {
      get: vi.fn(),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store });
    const publish = vi.fn();

    const unicodeTag = 'document:07-praetor-lite-modo-equipo-exploración';

    cache.allow();
    await expect(
      cache.revalidate({
        key: 'unicode-tag-key',
        payloadSchema,
        tags: ['workspace:workspace-a'],
        deriveTags: () => [unicodeTag],
        freshForMs: 30_000,
        retentionForMs: 60_000,
        load: vi.fn().mockResolvedValue({ title: 'Online' }),
        publish,
        isCurrent: () => true,
      }),
    ).resolves.toMatchObject({ published: true });

    expect(store.putMany).toHaveBeenCalledWith([
      expect.objectContaining({ tags: ['workspace:workspace-a', unicodeTag] }),
    ]);
  });

  it('keeps current active requests scheduled after an exact tag purge', async () => {
    let now = 0;
    const scheduled: Array<{ delay: number; callback: () => void }> = [];
    const principal = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const workspaceId = '019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const store = {
      get: vi.fn(),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      deleteScope: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({
      store,
      clock: { now: () => now },
      timer: {
        clear: vi.fn(),
        schedule: (delay, callback) => {
          scheduled.push({ delay, callback });
          return scheduled.length;
        },
      },
    });
    const catalogLoad = vi.fn().mockResolvedValue({ title: 'Catalog' });
    const bodyLoad = vi.fn().mockResolvedValue({ title: 'Body' });
    const backlinksLoad = vi.fn().mockResolvedValue({ title: 'Backlinks' });
    const request = (
      key: string,
      tags: string[],
      load: () => Promise<{ title: string }>,
    ): ResourceCacheRequest<{ title: string }> => ({
      key,
      payloadSchema,
      tags,
      freshForMs: 10,
      activeForMs: 10,
      retentionForMs: 60_000,
      load,
      publish: vi.fn(),
      isCurrent: () => true,
    });
    const catalog = request(
      `v1|p=${principal}|w=${workspaceId}|k=note-tree|r=project-a|q={}`,
      ['project:project-a'],
      catalogLoad,
    );
    const body = request(
      `v1|p=${principal}|w=${workspaceId}|k=note-body|r=note-a|q={}`,
      ['document:note-a'],
      bodyLoad,
    );
    const backlinks = request(
      `v1|p=${principal}|w=${workspaceId}|k=note-secondary|r=note-a|q={"type":"backlinks"}`,
      ['document:note-a', 'secondary:backlinks'],
      backlinksLoad,
    );

    cache.allow();
    cache.activate(catalog);
    cache.activate(body);
    cache.activate(backlinks);

    await expect(cache.purgeTags(['project:project-a'], principal, workspaceId)).resolves.toBe(true);

    now = 10;
    scheduled.at(-1)?.callback();
    await vi.waitFor(() => expect(bodyLoad).toHaveBeenCalledOnce());
    await vi.waitFor(() => expect(backlinksLoad).toHaveBeenCalledOnce());
    await vi.waitFor(() => expect(catalogLoad).toHaveBeenCalledOnce());
  });

  it('settles an exact tag purge without waiting for unrelated inflight work and fences matching late writes', async () => {
    let resolveMatchingLoad: ((payload: { title: string }) => void) | undefined;
    const principal = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const workspaceId = '019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const matchingKey = `v1|p=${principal}|w=${workspaceId}|k=note-body|r=matching|q={}`;
    const store = {
      get: vi.fn(),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      deleteScope: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({ store });
    const matchingPublish = vi.fn();

    cache.allow();
    void cache.revalidate({
      key: `v1|p=${principal}|w=${workspaceId}|k=note-body|r=unrelated|q={}`,
      payloadSchema,
      tags: ['document:unrelated'],
      freshForMs: 30_000,
      retentionForMs: 60_000,
      load: () => new Promise(() => undefined),
      publish: vi.fn(),
      isCurrent: () => true,
    });
    const matching = cache.revalidate({
      key: matchingKey,
      payloadSchema,
      tags: ['document:matching'],
      freshForMs: 30_000,
      retentionForMs: 60_000,
      load: () =>
        new Promise((resolve) => {
          resolveMatchingLoad = resolve;
        }),
      publish: matchingPublish,
      isCurrent: () => true,
    });

    await expect(cache.purgeTags(['document:matching'], principal, workspaceId)).resolves.toBe(true);

    resolveMatchingLoad?.({ title: 'Late matching response' });
    await matching;

    expect(store.deleteScope).toHaveBeenCalledWith({
      principal,
      workspaceId,
      tagsAny: ['document:matching'],
    });
    expect(store.putMany).not.toHaveBeenCalled();
    expect(matchingPublish).not.toHaveBeenCalled();
  });

  it('reactivates a current matching request after an exact tag purge without reviving a deactivated target', async () => {
    let now = 0;
    const scheduled: Array<{ delay: number; callback: () => void }> = [];
    const principal = 'user:019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const workspaceId = '019ef171-bbcf-7b90-9be6-5dbb382afd08';
    const store = {
      get: vi.fn(),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      deleteScope: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({
      store,
      clock: { now: () => now },
      timer: {
        clear: vi.fn(),
        schedule: (delay, callback) => {
          scheduled.push({ delay, callback });
          return scheduled.length;
        },
      },
    });
    const currentLoad = vi.fn().mockResolvedValue({ title: 'Current' });
    const formerLoad = vi.fn().mockResolvedValue({ title: 'Former' });
    const request = (key: string, load: () => Promise<{ title: string }>) => ({
      key,
      payloadSchema,
      tags: ['document:matching'],
      freshForMs: 10,
      activeForMs: 10,
      retentionForMs: 60_000,
      load,
      publish: vi.fn(),
      isCurrent: () => true,
    });

    cache.allow();
    const current = request(`v1|p=${principal}|w=${workspaceId}|k=note-body|r=current|q={}`, currentLoad);
    const former = request(`v1|p=${principal}|w=${workspaceId}|k=note-body|r=former|q={}`, formerLoad);
    cache.activate(current);
    cache.activate(former);
    cache.deactivate(former.key);

    await expect(cache.purgeTags(['document:matching'], principal, workspaceId)).resolves.toBe(true);

    now = 10;
    scheduled.at(-1)?.callback();
    await vi.waitFor(() => expect(currentLoad).toHaveBeenCalledOnce());
    expect(formerLoad).not.toHaveBeenCalled();
  });

  it('does not hydrate after the authorization lease expires', async () => {
    let now = 1_000;
    const store = {
      get: vi.fn().mockResolvedValue(createEntry('key-a', 'Cached', now)),
      putMany: vi.fn().mockResolvedValue(true),
      deleteMany: vi.fn().mockResolvedValue(true),
      clear: vi.fn().mockResolvedValue(true),
    };
    const cache = new ResourceCache({
      store,
      clock: { now: () => now },
      policy: {
        enabled: true,
        authorizationLeaseMs: 10,
        hot: { maxEntries: 100 },
        persistent: { maxBytes: 1, maxEntries: 1, maxNoteBodyBytes: 1, maxOtherEntryBytes: 1 },
      },
    });
    const publish = vi.fn();

    cache.allow();
    now += 11;
    const hydrated = await cache.hydrate({ key: 'key-a', payloadSchema, publish, isCurrent: () => true });

    expect(hydrated).toBeNull();
    expect(publish).not.toHaveBeenCalled();
  });
});
