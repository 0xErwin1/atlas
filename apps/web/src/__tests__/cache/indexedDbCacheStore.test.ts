import { describe, expect, it } from 'vitest';
import { z } from 'zod';
import {
  configureResourceCacheForTest,
  runHardRefresh,
  setResourceCachePrincipal,
} from '@/cache/cacheRuntime';
import { evictEntries, IndexedDbCacheStore, validatePersistedEnvelope } from '@/cache/indexedDbCacheStore';
import { buildCacheKey, type CacheEnvelope, createCacheEnvelope, ResourceCache } from '@/cache/resourceCache';

const payloadSchema = z.object({ title: z.string() });
const workspaceId = '018f8e6d-7c15-7c72-8a41-2f5295e0c0f1';
const principal = 'user:018f8e6d-7c15-7c72-8a41-2f5295e0c0f2';

function entry(key: string, overrides: Partial<CacheEnvelope<{ title: string }>> = {}) {
  return createCacheEnvelope({
    key,
    payloadVersion: 1,
    storedAt: 1,
    validatedAt: 1,
    lastAccessedAt: 1,
    retentionExpiresAt: 100,
    bytes: 5,
    stale: false,
    tags: [],
    payload: { title: key },
    ...overrides,
  });
}

function keyFor(resourceId: string) {
  const key = buildCacheKey({ principal, workspaceId, resourceKind: 'task-detail', resourceId });

  if (!key) {
    throw new Error('test cache key must be valid');
  }

  return key;
}

class FakeIndexedDbFactory {
  readonly databases = new Map<string, { entries: Map<string, unknown>; version: number }>();
  deleteCount = 0;
  blockedDeleteOutcome: 'error' | 'success' | null = null;
  failDeletes = false;
  failWrites = false;

  asIdbFactory(): IDBFactory {
    return this as unknown as IDBFactory;
  }

  open(name: string, version: number) {
    const request: {
      error: DOMException | null;
      result: unknown;
      onerror: (() => void) | null;
      onsuccess: (() => void) | null;
      onupgradeneeded: (() => void) | null;
    } = {
      error: null,
      result: undefined,
      onerror: null,
      onsuccess: null,
      onupgradeneeded: null,
    };

    queueMicrotask(() => {
      const existing = this.databases.get(name);

      if (existing && existing.version > version) {
        request.error = new DOMException('Database version is newer', 'VersionError');
        request.onerror?.();
        return;
      }

      const database = existing ?? { entries: new Map<string, unknown>(), version };
      this.databases.set(name, database);
      request.result = new FakeDatabase(database, this);

      if (!existing) {
        request.onupgradeneeded?.();
      }

      request.onsuccess?.();
    });

    return request;
  }

  deleteDatabase(name: string) {
    const request: {
      onblocked: (() => void) | null;
      onerror: (() => void) | null;
      onsuccess: (() => void) | null;
    } = {
      onblocked: null,
      onerror: null,
      onsuccess: null,
    };

    queueMicrotask(() => {
      this.deleteCount += 1;

      if (this.blockedDeleteOutcome) {
        request.onblocked?.();

        queueMicrotask(() => {
          if (this.blockedDeleteOutcome === 'error') {
            request.onerror?.();
            return;
          }

          this.databases.delete(name);
          request.onsuccess?.();
        });
        return;
      }

      this.databases.delete(name);
      request.onsuccess?.();
    });

    return request;
  }
}

class FakeDatabase {
  constructor(
    private readonly database: { entries: Map<string, unknown>; version: number },
    private readonly factory: FakeIndexedDbFactory,
  ) {}

  get objectStoreNames() {
    return { contains: () => true };
  }

  createObjectStore() {
    return undefined;
  }

  deleteObjectStore() {
    this.database.entries.clear();
  }

  transaction() {
    return new FakeTransaction(this.database.entries, this.factory);
  }
}

class FakeTransaction {
  onabort: (() => void) | null = null;
  oncomplete: (() => void) | null = null;
  onerror: (() => void) | null = null;
  private aborted = false;
  private readonly changes = new Map<string, unknown | undefined>();

  constructor(
    private readonly entries: Map<string, unknown>,
    private readonly factory: FakeIndexedDbFactory,
  ) {
    setTimeout(() => this.finish(), 0);
  }

  objectStore() {
    return {
      clear: () => this.changes.clear(),
      delete: (key: string) => {
        if (this.factory.failDeletes) {
          this.abort();
          return;
        }

        this.changes.set(key, undefined);
      },
      get: (key: string) => this.request(this.entries.get(key)),
      getAll: (_query?: IDBValidKey | IDBKeyRange | null, count?: number) =>
        this.request([...this.entries.values()].slice(0, count)),
      put: (value: { key: string }) => {
        if (this.factory.failWrites) {
          this.abort();
          return;
        }

        this.changes.set(value.key, value);
      },
    };
  }

  abort() {
    this.aborted = true;
  }

  private finish() {
    if (this.aborted) {
      this.onabort?.();
      return;
    }

    for (const [key, value] of this.changes) {
      if (value === undefined) {
        this.entries.delete(key);
      } else {
        this.entries.set(key, value);
      }
    }

    this.oncomplete?.();
  }

  private request(result: unknown) {
    const request: {
      error: DOMException | null;
      result: unknown;
      onerror: (() => void) | null;
      onsuccess: (() => void) | null;
    } = {
      error: null,
      result,
      onerror: null,
      onsuccess: null,
    };

    queueMicrotask(() => request.onsuccess?.());
    return request;
  }
}

describe('IndexedDbCacheStore contracts', () => {
  it('atomically deletes only cold entries matching a principal workspace and tag scope', async () => {
    const indexedDb = new FakeIndexedDbFactory();
    const matchingKey = keyFor('matching');
    const unrelatedKey = keyFor('unrelated');
    const otherPrincipal = buildCacheKey({
      principal: 'user:018f8e6d-7c15-7c72-8a41-2f5295e0c0f3',
      workspaceId,
      resourceKind: 'task-detail',
      resourceId: 'other-principal',
    });

    if (otherPrincipal === null) throw new Error('test cache key must be valid');

    indexedDb.databases.set('scoped-delete', {
      entries: new Map([
        [matchingKey, entry(matchingKey, { tags: ['document:matching'] })],
        [unrelatedKey, entry(unrelatedKey, { tags: ['document:unrelated'] })],
        [otherPrincipal, entry(otherPrincipal, { tags: ['document:matching'] })],
      ]),
      version: 1,
    });
    const store = new IndexedDbCacheStore({
      indexedDb: indexedDb.asIdbFactory(),
      databaseName: 'scoped-delete',
    });

    await expect(store.deleteScope({ principal, workspaceId, tagsAny: ['document:matching'] })).resolves.toBe(
      true,
    );
    expect(indexedDb.databases.get('scoped-delete')?.entries.has(matchingKey)).toBe(false);
    expect(indexedDb.databases.get('scoped-delete')?.entries.has(unrelatedKey)).toBe(true);
    expect(indexedDb.databases.get('scoped-delete')?.entries.has(otherPrincipal)).toBe(true);
  });

  it('fails closed without deleting records when matching entries exceed the scan bound', async () => {
    const indexedDb = new FakeIndexedDbFactory();
    const keys = Array.from({ length: 501 }, (_, index) => keyFor(`bounded-${index}`));
    const unrelatedKey = keyFor('bounded-unrelated');
    indexedDb.databases.set('bounded-delete', {
      entries: new Map([
        ...keys.map((key) => [key, entry(key, { tags: ['document:matching'] })] as const),
        [unrelatedKey, entry(unrelatedKey, { tags: ['document:unrelated'] })],
      ]),
      version: 1,
    });
    const store = new IndexedDbCacheStore({
      indexedDb: indexedDb.asIdbFactory(),
      databaseName: 'bounded-delete',
    });

    await expect(store.deleteScope({ principal, workspaceId, tagsAny: ['document:matching'] })).resolves.toBe(
      false,
    );
    expect(indexedDb.databases.get('bounded-delete')?.entries.has(keys[0] ?? '')).toBe(true);
    expect(indexedDb.databases.get('bounded-delete')?.entries.has(keys[499] ?? '')).toBe(true);
    expect(indexedDb.databases.get('bounded-delete')?.entries.has(keys[500] ?? '')).toBe(true);
    expect(indexedDb.databases.get('bounded-delete')?.entries.has(unrelatedKey)).toBe(true);
  });

  it('fails closed when a matching record is beyond the first scan page', async () => {
    const indexedDb = new FakeIndexedDbFactory();
    const unrelatedKeys = Array.from({ length: 500 }, (_, index) => keyFor(`unrelated-${index}`));
    const matchingKey = keyFor('after-first-page');
    indexedDb.databases.set('beyond-page-delete', {
      entries: new Map([
        ...unrelatedKeys.map((key) => [key, entry(key, { tags: ['document:unrelated'] })] as const),
        [matchingKey, entry(matchingKey, { tags: ['document:matching'] })],
      ]),
      version: 1,
    });
    const store = new IndexedDbCacheStore({
      indexedDb: indexedDb.asIdbFactory(),
      databaseName: 'beyond-page-delete',
    });

    await expect(store.deleteScope({ principal, workspaceId, tagsAny: ['document:matching'] })).resolves.toBe(
      false,
    );
    expect(indexedDb.databases.get('beyond-page-delete')?.entries.has(matchingKey)).toBe(true);
    expect(indexedDb.databases.get('beyond-page-delete')?.entries.has(unrelatedKeys[499] ?? '')).toBe(true);
  });

  it('fails closed and preserves all records when scoped deletion aborts', async () => {
    const indexedDb = new FakeIndexedDbFactory();
    const matchingKey = keyFor('failed-delete');
    const unrelatedKey = keyFor('failed-delete-unrelated');
    indexedDb.databases.set('failed-scope-delete', {
      entries: new Map([
        [matchingKey, entry(matchingKey, { tags: ['document:matching'] })],
        [unrelatedKey, entry(unrelatedKey, { tags: ['document:unrelated'] })],
      ]),
      version: 1,
    });
    indexedDb.failDeletes = true;
    const store = new IndexedDbCacheStore({
      indexedDb: indexedDb.asIdbFactory(),
      databaseName: 'failed-scope-delete',
    });

    await expect(store.deleteScope({ principal, workspaceId, tagsAny: ['document:matching'] })).resolves.toBe(
      false,
    );
    expect(indexedDb.databases.get('failed-scope-delete')?.entries.has(matchingKey)).toBe(true);
    expect(indexedDb.databases.get('failed-scope-delete')?.entries.has(unrelatedKey)).toBe(true);
  });

  it('executes successful transactions and fails closed on storage failure', async () => {
    const indexedDb = new FakeIndexedDbFactory();
    const store = new IndexedDbCacheStore({
      indexedDb: indexedDb.asIdbFactory(),
      databaseName: 'transactions',
      now: () => 9,
    });
    const valid = entry(keyFor('task-a'));

    await expect(store.putMany([valid])).resolves.toBe(true);
    await expect(store.get(valid.key, payloadSchema)).resolves.toMatchObject({
      payload: { title: valid.key },
      lastAccessedAt: 9,
    });

    indexedDb.failWrites = true;
    await expect(store.putMany([entry(keyFor('task-b'))])).resolves.toBe(false);
    await expect(store.get(keyFor('task-b'), payloadSchema)).resolves.toBeNull();
  });

  it('waits for corrupt-record deletion and fails closed when that transaction aborts', async () => {
    const indexedDb = new FakeIndexedDbFactory();
    const key = keyFor('corrupt');
    indexedDb.databases.set('corruption', {
      entries: new Map([[key, { ...entry(key), payload: { title: 12 } }]]),
      version: 1,
    });
    const store = new IndexedDbCacheStore({
      indexedDb: indexedDb.asIdbFactory(),
      databaseName: 'corruption',
    });

    await expect(store.get(key, payloadSchema)).resolves.toBeNull();
    expect(indexedDb.databases.get('corruption')?.entries.has(key)).toBe(false);

    indexedDb.databases.get('corruption')?.entries.set(key, { ...entry(key), payload: { title: 12 } });
    indexedDb.failDeletes = true;
    await expect(store.get(key, payloadSchema)).resolves.toBeNull();
    expect(indexedDb.databases.get('corruption')?.entries.has(key)).toBe(true);
  });

  it('never returns retention-expired content and waits for its deletion to commit', async () => {
    const indexedDb = new FakeIndexedDbFactory();
    const key = keyFor('expired-read');
    indexedDb.databases.set('retention-read', {
      entries: new Map([[key, entry(key, { retentionExpiresAt: 9 })]]),
      version: 1,
    });
    const store = new IndexedDbCacheStore({
      indexedDb: indexedDb.asIdbFactory(),
      databaseName: 'retention-read',
      now: () => 10,
    });

    await expect(store.get(key, payloadSchema)).resolves.toBeNull();
    expect(indexedDb.databases.get('retention-read')?.entries.has(key)).toBe(false);

    indexedDb.databases.get('retention-read')?.entries.set(key, entry(key, { retentionExpiresAt: 9 }));
    indexedDb.failDeletes = true;
    await expect(store.get(key, payloadSchema)).resolves.toBeNull();
    expect(indexedDb.databases.get('retention-read')?.entries.has(key)).toBe(true);
  });

  it('purges incompatible databases and distinguishes retention expiry from stale freshness', async () => {
    const indexedDb = new FakeIndexedDbFactory();
    indexedDb.databases.set('versioned', {
      entries: new Map([[keyFor('legacy'), entry(keyFor('legacy'))]]),
      version: 2,
    });
    const versionedStore = new IndexedDbCacheStore({
      indexedDb: indexedDb.asIdbFactory(),
      databaseName: 'versioned',
      now: () => 10,
    });
    const stale = entry(keyFor('stale'), { stale: true, retentionExpiresAt: 100 });
    const expired = entry(keyFor('expired'), { retentionExpiresAt: 9 });

    await expect(versionedStore.putMany([stale, expired])).resolves.toBe(true);
    expect(indexedDb.deleteCount).toBe(1);
    await expect(versionedStore.get(stale.key, payloadSchema)).resolves.toMatchObject({
      payload: { title: stale.key },
    });
    await expect(versionedStore.get(expired.key, payloadSchema)).resolves.toBeNull();
  });

  it('waits for a blocked purge terminal result and retries after a terminal purge failure', async () => {
    const indexedDb = new FakeIndexedDbFactory();
    indexedDb.databases.set('blocked-success', {
      entries: new Map([[keyFor('legacy-success'), entry(keyFor('legacy-success'))]]),
      version: 2,
    });
    indexedDb.blockedDeleteOutcome = 'success';
    const successfulStore = new IndexedDbCacheStore({
      indexedDb: indexedDb.asIdbFactory(),
      databaseName: 'blocked-success',
      now: () => 10,
    });

    await expect(successfulStore.putMany([entry(keyFor('fresh-success'))])).resolves.toBe(true);
    expect(indexedDb.databases.get('blocked-success')?.entries.has(keyFor('fresh-success'))).toBe(true);

    indexedDb.databases.set('blocked-error', {
      entries: new Map([[keyFor('legacy-error'), entry(keyFor('legacy-error'))]]),
      version: 2,
    });
    indexedDb.blockedDeleteOutcome = 'error';
    const retryingStore = new IndexedDbCacheStore({
      indexedDb: indexedDb.asIdbFactory(),
      databaseName: 'blocked-error',
      now: () => 10,
    });

    await expect(retryingStore.putMany([entry(keyFor('first-attempt'))])).resolves.toBe(false);
    indexedDb.blockedDeleteOutcome = null;
    await expect(retryingStore.putMany([entry(keyFor('second-attempt'))])).resolves.toBe(true);
    expect(indexedDb.databases.get('blocked-error')?.entries.has(keyFor('second-attempt'))).toBe(true);
  });

  it('rejects corrupt payloads and evicts by expiry, activity, LRU, then lexical key', () => {
    const invalid = entry(keyFor('invalid'), { payload: { title: 12 } as unknown as { title: string } });

    expect(validatePersistedEnvelope(invalid, payloadSchema)).toBeNull();
    expect(
      evictEntries(
        [
          entry('active-b', { lastAccessedAt: 4 }),
          entry('inactive', { lastAccessedAt: 3 }),
          entry('expired', { lastAccessedAt: 10 }),
          entry('active-a', { lastAccessedAt: 4 }),
        ],
        {
          activeKeys: new Set(['active-b', 'active-a']),
          expiredKeys: new Set(['expired']),
          maxEntries: 1,
          maxBytes: 5,
        },
      ).map((candidate) => candidate.key),
    ).toEqual(['expired', 'inactive', 'active-a']);
  });

  it('removes the actual hot and IndexedDB cold envelope before hard refresh reloads', async () => {
    const indexedDb = new FakeIndexedDbFactory();
    const store = new IndexedDbCacheStore({
      indexedDb: indexedDb.asIdbFactory(),
      databaseName: 'hard-refresh',
    });
    const cache = new ResourceCache({ store });
    const key = keyFor('hard-refresh');
    const reload = async () => {
      expect(await store.get(key, payloadSchema)).toBeNull();
      expect(
        await cache.hydrate({ key, payloadSchema, publish: () => undefined, isCurrent: () => true }),
      ).toBeNull();
    };

    cache.allow();
    await cache.revalidate({
      key,
      payloadSchema,
      tags: [`workspace:${workspaceId}`, 'document:hard-refresh'],
      freshForMs: 30_000,
      retentionForMs: 60_000,
      load: async () => ({ title: 'Hot and cold' }),
      publish: () => undefined,
      isCurrent: () => true,
    });
    configureResourceCacheForTest(cache);
    setResourceCachePrincipal(principal);

    await expect(runHardRefresh(workspaceId, reload)).resolves.toBe(true);
  });
});
