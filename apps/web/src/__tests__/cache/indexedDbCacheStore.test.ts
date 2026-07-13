import { describe, expect, it } from 'vitest';
import { z } from 'zod';
import { evictEntries, IndexedDbCacheStore, validatePersistedEnvelope } from '@/cache/indexedDbCacheStore';
import { buildCacheKey, type CacheEnvelope, createCacheEnvelope } from '@/cache/resourceCache';

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
      getAll: () => this.request([...this.entries.values()]),
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
});
