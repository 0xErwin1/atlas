import { describe, expect, it } from 'vitest';
import { z } from 'zod';
import {
  configureResourceCacheForTest,
  runHardRefresh,
  setResourceCachePrincipal,
} from '@/cache/cacheRuntime';
import {
  CACHE_ENTRY_INDEX_STORE_NAME,
  CACHE_ENTRY_STORE_NAME,
  createCacheEntryIndexRecord,
  evictEntries,
  IndexedDbCacheStore,
  validatePersistedEnvelope,
} from '@/cache/indexedDbCacheStore';
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

class FakeDatabaseState {
  readonly stores = new Map<string, Map<string, unknown>>([
    [CACHE_ENTRY_STORE_NAME, new Map<string, unknown>()],
    [CACHE_ENTRY_INDEX_STORE_NAME, new Map<string, unknown>()],
  ]);

  constructor(public version: number) {}

  get entries(): Map<string, unknown> {
    return this.store(CACHE_ENTRY_STORE_NAME);
  }

  get entryIndex(): Map<string, unknown> {
    return this.store(CACHE_ENTRY_INDEX_STORE_NAME);
  }

  store(name: string): Map<string, unknown> {
    const store = this.stores.get(name);

    if (!store) {
      throw new Error(`unknown object store ${name}`);
    }

    return store;
  }
}

function seededDatabase(envelopes: readonly CacheEnvelope<unknown>[], version = 2): FakeDatabaseState {
  const state = new FakeDatabaseState(version);

  for (const envelope of envelopes) {
    state.entries.set(envelope.key, envelope);
    state.entryIndex.set(envelope.key, createCacheEntryIndexRecord(envelope));
  }

  return state;
}

class FakeIndexedDbFactory {
  readonly databases = new Map<string, FakeDatabaseState>();
  private readonly reads = new Map<string, number>();
  deleteCount = 0;
  blockedDeleteOutcome: 'error' | 'success' | null = null;
  failDeletes = false;
  failWrites = false;

  asIdbFactory(): IDBFactory {
    return this as unknown as IDBFactory;
  }

  recordRead(storeName: string, count: number): void {
    this.reads.set(storeName, (this.reads.get(storeName) ?? 0) + count);
  }

  readCount(storeName: string): number {
    return this.reads.get(storeName) ?? 0;
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

      const database = existing ?? new FakeDatabaseState(version);
      const upgrading = !existing || existing.version < version;
      database.version = version;
      this.databases.set(name, database);
      request.result = new FakeDatabase(database, this);

      if (upgrading) {
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
    private readonly database: FakeDatabaseState,
    private readonly factory: FakeIndexedDbFactory,
  ) {}

  get objectStoreNames() {
    return { contains: (name: string) => this.database.stores.has(name) };
  }

  createObjectStore(name: string) {
    this.database.stores.set(name, new Map<string, unknown>());
    return undefined;
  }

  deleteObjectStore(name: string) {
    this.database.stores.delete(name);
  }

  transaction() {
    return new FakeTransaction(this.database, this.factory);
  }
}

class FakeTransaction {
  onabort: (() => void) | null = null;
  oncomplete: (() => void) | null = null;
  onerror: (() => void) | null = null;
  private aborted = false;
  private readonly changes = new Map<string, Map<string, unknown | undefined>>();

  constructor(
    private readonly database: FakeDatabaseState,
    private readonly factory: FakeIndexedDbFactory,
  ) {
    setTimeout(() => this.finish(), 0);
  }

  objectStore(name: string) {
    const entries = this.database.store(name);
    const changes = this.changesFor(name);

    return {
      clear: () => changes.clear(),
      delete: (key: string) => {
        if (this.factory.failDeletes) {
          this.abort();
          return;
        }

        changes.set(key, undefined);
      },
      get: (key: string) => {
        this.factory.recordRead(name, 1);
        return this.request(entries.get(key));
      },
      getAll: (_query?: IDBValidKey | IDBKeyRange | null, count?: number) => {
        const values = [...entries.values()].slice(0, count);
        this.factory.recordRead(name, values.length);
        return this.request(values);
      },
      put: (value: { key: string }) => {
        if (this.factory.failWrites) {
          this.abort();
          return;
        }

        changes.set(value.key, value);
      },
    };
  }

  abort() {
    this.aborted = true;
  }

  private changesFor(name: string): Map<string, unknown | undefined> {
    const pending = this.changes.get(name) ?? new Map<string, unknown | undefined>();
    this.changes.set(name, pending);
    return pending;
  }

  private finish() {
    if (this.aborted) {
      this.onabort?.();
      return;
    }

    for (const [name, pending] of this.changes) {
      const entries = this.database.store(name);

      for (const [key, value] of pending) {
        if (value === undefined) {
          entries.delete(key);
        } else {
          entries.set(key, value);
        }
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

    indexedDb.databases.set(
      'scoped-delete',
      seededDatabase([
        entry(matchingKey, { tags: ['document:matching'] }),
        entry(unrelatedKey, { tags: ['document:unrelated'] }),
        entry(otherPrincipal, { tags: ['document:matching'] }),
      ]),
    );
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
    indexedDb.databases.set(
      'bounded-delete',
      seededDatabase([
        ...keys.map((key) => entry(key, { tags: ['document:matching'] })),
        entry(unrelatedKey, { tags: ['document:unrelated'] }),
      ]),
    );
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
    indexedDb.databases.set(
      'beyond-page-delete',
      seededDatabase([
        ...unrelatedKeys.map((key) => entry(key, { tags: ['document:unrelated'] })),
        entry(matchingKey, { tags: ['document:matching'] }),
      ]),
    );
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
    indexedDb.databases.set(
      'failed-scope-delete',
      seededDatabase([
        entry(matchingKey, { tags: ['document:matching'] }),
        entry(unrelatedKey, { tags: ['document:unrelated'] }),
      ]),
    );
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
    indexedDb.databases.set(
      'corruption',
      seededDatabase([{ ...entry(key), payload: { title: 12 } } as CacheEnvelope<unknown>]),
    );
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
    indexedDb.databases.set('retention-read', seededDatabase([entry(key, { retentionExpiresAt: 9 })]));
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
    indexedDb.databases.set('versioned', seededDatabase([entry(keyFor('legacy'))], 3));
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
    indexedDb.databases.set('blocked-success', seededDatabase([entry(keyFor('legacy-success'))], 3));
    indexedDb.blockedDeleteOutcome = 'success';
    const successfulStore = new IndexedDbCacheStore({
      indexedDb: indexedDb.asIdbFactory(),
      databaseName: 'blocked-success',
      now: () => 10,
    });

    await expect(successfulStore.putMany([entry(keyFor('fresh-success'))])).resolves.toBe(true);
    expect(indexedDb.databases.get('blocked-success')?.entries.has(keyFor('fresh-success'))).toBe(true);

    indexedDb.databases.set('blocked-error', seededDatabase([entry(keyFor('legacy-error'))], 3));
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

  it('writes and scope-purges without materializing any stored payload', async () => {
    const indexedDb = new FakeIndexedDbFactory();
    const existing = Array.from({ length: 20 }, (_, index) => entry(keyFor(`existing-${index}`)));
    indexedDb.databases.set('metadata-only', seededDatabase(existing));
    const store = new IndexedDbCacheStore({
      indexedDb: indexedDb.asIdbFactory(),
      databaseName: 'metadata-only',
    });

    await expect(store.putMany([entry(keyFor('written'))])).resolves.toBe(true);
    await expect(store.deleteScope({ principal, workspaceId })).resolves.toBe(true);

    expect(indexedDb.readCount(CACHE_ENTRY_STORE_NAME)).toBe(0);
    expect(indexedDb.readCount(CACHE_ENTRY_INDEX_STORE_NAME)).toBeGreaterThan(0);
    expect(indexedDb.databases.get('metadata-only')?.entries.size).toBe(0);
    expect(indexedDb.databases.get('metadata-only')?.entryIndex.size).toBe(0);
  });

  it('evicts the least recently used cold entry and its index record when over the entry budget', async () => {
    const indexedDb = new FakeIndexedDbFactory();
    const oldest = entry(keyFor('oldest'), { lastAccessedAt: 1 });
    const newer = entry(keyFor('newer'), { lastAccessedAt: 5 });
    indexedDb.databases.set('eviction', seededDatabase([oldest, newer]));
    const store = new IndexedDbCacheStore({
      indexedDb: indexedDb.asIdbFactory(),
      databaseName: 'eviction',
      limits: { maxBytes: 1_000, maxEntries: 2, maxNoteBodyBytes: 1_000, maxOtherEntryBytes: 1_000 },
      now: () => 10,
    });
    const written = entry(keyFor('written'), { lastAccessedAt: 9 });

    await expect(store.putMany([written])).resolves.toBe(true);

    const database = indexedDb.databases.get('eviction');
    expect(database?.entries.has(oldest.key)).toBe(false);
    expect(database?.entryIndex.has(oldest.key)).toBe(false);
    expect(database?.entries.has(newer.key)).toBe(true);
    expect(database?.entries.has(written.key)).toBe(true);
    expect(database?.entryIndex.has(written.key)).toBe(true);
  });

  it('keeps the index record in step with a read that refreshes last access', async () => {
    const indexedDb = new FakeIndexedDbFactory();
    const key = keyFor('touched');
    indexedDb.databases.set('index-sync', seededDatabase([entry(key)]));
    const store = new IndexedDbCacheStore({
      indexedDb: indexedDb.asIdbFactory(),
      databaseName: 'index-sync',
      now: () => 42,
    });

    await expect(store.get(key, payloadSchema)).resolves.toMatchObject({ lastAccessedAt: 42 });
    expect(indexedDb.databases.get('index-sync')?.entryIndex.get(key)).toMatchObject({
      key,
      lastAccessedAt: 42,
    });
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
