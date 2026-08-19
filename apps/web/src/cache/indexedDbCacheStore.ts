import { type ZodType, z } from 'zod';
import {
  type CacheDeleteScope,
  type CacheEnvelope,
  type CacheLimits,
  createCacheEnvelopeSchema,
  DEFAULT_CACHE_POLICY,
} from './resourceCache';

const DATABASE_NAME = 'atlas-resource-cache';
const DATABASE_VERSION = 2;

export const CACHE_ENTRY_STORE_NAME = 'entries';
export const CACHE_ENTRY_INDEX_STORE_NAME = 'entryIndex';

const STORE_NAMES = [CACHE_ENTRY_STORE_NAME, CACHE_ENTRY_INDEX_STORE_NAME] as const;

/**
 * The payload-free projection of a cached envelope.
 *
 * Eviction and scoped deletion only ever need a key, its size, its recency and
 * its tags, so they read this sidecar store instead of the envelope store. That
 * is what keeps a single write from structured-cloning every cached payload —
 * up to the full 50 MB budget — onto the main thread.
 *
 * It is written and deleted in the same transaction as the envelope it
 * describes, so the two stores cannot drift, not even across tabs.
 */
export interface CacheEntryIndexRecord {
  key: string;
  bytes: number;
  lastAccessedAt: number;
  retentionExpiresAt: number;
  tags: string[];
}

const cacheEntryIndexRecordSchema = z.object({
  key: z.string().min(1),
  bytes: z.number().finite().nonnegative(),
  lastAccessedAt: z.number().finite().nonnegative(),
  retentionExpiresAt: z.number().finite().nonnegative(),
  tags: z.array(z.string()),
});

interface EvictionOptions {
  activeKeys: ReadonlySet<string>;
  expiredKeys: ReadonlySet<string>;
  maxEntries: number;
  maxBytes: number;
}

export interface IndexedDbCacheStoreOptions {
  activeKeys?: () => ReadonlySet<string>;
  databaseName?: string;
  indexedDb?: IDBFactory;
  limits?: CacheLimits;
  now?: () => number;
}

export class IndexedDbCacheStore {
  private readonly activeKeys: () => ReadonlySet<string>;
  private readonly databaseName: string;
  private readonly indexedDb: IDBFactory | undefined;
  private readonly limits: CacheLimits;
  private readonly now: () => number;
  private databasePromise: Promise<IDBDatabase | null> | null = null;
  private available = true;

  constructor(options: IndexedDbCacheStoreOptions = {}) {
    this.activeKeys = options.activeKeys ?? (() => new Set());
    this.databaseName = options.databaseName ?? DATABASE_NAME;
    this.indexedDb = options.indexedDb ?? globalThis.indexedDB;
    this.limits = options.limits ?? DEFAULT_CACHE_POLICY.persistent;
    this.now = options.now ?? Date.now;
  }

  async get<T>(key: string, payloadSchema: ZodType<T>): Promise<CacheEnvelope<T> | null> {
    const database = await this.open();

    if (!database) {
      return null;
    }

    try {
      const transaction = database.transaction(STORE_NAMES, 'readwrite');
      const store = transaction.objectStore(CACHE_ENTRY_STORE_NAME);
      const entryIndex = transaction.objectStore(CACHE_ENTRY_INDEX_STORE_NAME);
      const request = store.get(key);

      return new Promise((resolve) => {
        let entry: CacheEnvelope<T> | null = null;
        let settled = false;
        const complete = (result: CacheEnvelope<T> | null) => {
          if (!settled) {
            settled = true;
            resolve(result);
          }
        };

        request.onerror = () => transaction.abort();
        request.onsuccess = () => {
          const persisted = validatePersistedEnvelope(request.result, payloadSchema);

          if (!persisted) {
            if (request.result !== undefined) {
              store.delete(key);
              entryIndex.delete(key);
            }
            return;
          }

          if (persisted.retentionExpiresAt <= this.now()) {
            store.delete(key);
            entryIndex.delete(key);
            return;
          }

          persisted.lastAccessedAt = this.now();
          entry = persisted;
          store.put(persisted);
          entryIndex.put(createCacheEntryIndexRecord(persisted));
        };
        transaction.oncomplete = () => complete(entry);
        transaction.onabort = () => complete(null);
        transaction.onerror = () => complete(null);
      });
    } catch {
      return null;
    }
  }

  async putMany(entries: readonly CacheEnvelope<unknown>[]): Promise<boolean> {
    if (!entries.every((entry) => isStorableEntry(entry, this.limits))) {
      return false;
    }

    const database = await this.open();

    if (!database) {
      return false;
    }

    const transaction = database.transaction(STORE_NAMES, 'readwrite');
    const store = transaction.objectStore(CACHE_ENTRY_STORE_NAME);
    const entryIndex = transaction.objectStore(CACHE_ENTRY_INDEX_STORE_NAME);
    const request = entryIndex.getAll();

    return new Promise((resolve) => {
      let settled = false;
      const complete = (result: boolean) => {
        if (!settled) {
          settled = true;
          resolve(result);
        }
      };

      const drop = (key: string) => {
        store.delete(key);
        entryIndex.delete(key);
      };

      request.onerror = () => transaction.abort();
      request.onsuccess = () => {
        const persisted: CacheEntryIndexRecord[] = [];

        for (const candidate of request.result) {
          const record = validateIndexRecord(candidate);

          if (record) {
            persisted.push(record);
            continue;
          }

          const key = cacheKeyOf(candidate);
          if (key) drop(key);
        }

        const entryKeys = new Set(entries.map((entry) => entry.key));
        const existing = persisted.filter((record) => !entryKeys.has(record.key));
        const candidates = [...existing, ...entries.map(createCacheEntryIndexRecord)];
        const evicted = evictEntries(candidates, {
          activeKeys: this.activeKeys(),
          expiredKeys: new Set(
            candidates
              .filter((record) => record.retentionExpiresAt <= this.now())
              .map((record) => record.key),
          ),
          maxEntries: this.limits.maxEntries,
          maxBytes: this.limits.maxBytes,
        });

        for (const record of evicted) {
          drop(record.key);
        }

        const evictedKeys = new Set(evicted.map((record) => record.key));
        for (const entry of entries) {
          if (!evictedKeys.has(entry.key)) {
            store.put(entry);
            entryIndex.put(createCacheEntryIndexRecord(entry));
          }
        }
      };
      transaction.oncomplete = () => complete(true);
      transaction.onabort = () => complete(false);
      transaction.onerror = () => complete(false);
    });
  }

  async deleteMany(keys: readonly string[]): Promise<boolean> {
    return this.mutate((store, entryIndex) => {
      for (const key of keys) {
        store.delete(key);
        entryIndex.delete(key);
      }
    });
  }

  async deleteScope(scope: CacheDeleteScope): Promise<boolean> {
    const database = await this.open();

    if (!database) return false;

    try {
      const transaction = database.transaction(STORE_NAMES, 'readwrite');
      const store = transaction.objectStore(CACHE_ENTRY_STORE_NAME);
      const entryIndex = transaction.objectStore(CACHE_ENTRY_INDEX_STORE_NAME);
      const request = entryIndex.getAll(null, this.limits.maxEntries + 1);

      return new Promise((resolve) => {
        let settled = false;
        const complete = (result: boolean) => {
          if (!settled) {
            settled = true;
            resolve(result);
          }
        };

        request.onerror = () => transaction.abort();
        request.onsuccess = () => {
          if (request.result.length > this.limits.maxEntries) {
            transaction.abort();
            return;
          }

          for (const candidate of request.result) {
            if (!matchesScope(candidate, scope)) continue;

            store.delete(candidate.key);
            entryIndex.delete(candidate.key);
          }
        };
        transaction.oncomplete = () => complete(true);
        transaction.onabort = () => complete(false);
        transaction.onerror = () => complete(false);
      });
    } catch {
      return false;
    }
  }

  async clear(): Promise<boolean> {
    return this.mutate((store, entryIndex) => {
      store.clear();
      entryIndex.clear();
    });
  }

  private async mutate(
    operation: (store: IDBObjectStore, entryIndex: IDBObjectStore) => void,
  ): Promise<boolean> {
    const database = await this.open();

    if (!database) {
      return false;
    }

    const transaction = database.transaction(STORE_NAMES, 'readwrite');

    try {
      operation(
        transaction.objectStore(CACHE_ENTRY_STORE_NAME),
        transaction.objectStore(CACHE_ENTRY_INDEX_STORE_NAME),
      );
    } catch {
      transaction.abort();
      return false;
    }

    return transactionResult(transaction);
  }

  /**
   * Whether this store has proven usable. Starts optimistic and only turns false
   * once an open attempt has actually failed, so callers must read it after
   * awaiting a store operation rather than before.
   *
   * A store that reports false can neither persist nor serve anything, which is
   * what lets the resource cache keep working from memory instead of treating
   * the failure as a correctness hazard.
   */
  isAvailable(): boolean {
    return this.available;
  }

  private open(): Promise<IDBDatabase | null> {
    const indexedDb = this.indexedDb;

    if (!indexedDb) {
      this.available = false;
      return Promise.resolve(null);
    }

    if (this.databasePromise) {
      return this.databasePromise;
    }

    this.databasePromise = this.openDatabase()
      .catch(async (error) => {
        if (isVersionError(error) && (await this.purgeDatabase())) {
          return this.openDatabase();
        }

        throw error;
      })
      .then((database) => {
        this.available = true;
        return database;
      })
      .catch(() => {
        this.databasePromise = null;
        this.available = false;
        return null;
      });

    return this.databasePromise;
  }

  private openDatabase(): Promise<IDBDatabase> {
    const indexedDb = this.indexedDb;

    if (!indexedDb) {
      return Promise.reject(new Error('IndexedDB is unavailable.'));
    }

    return new Promise<IDBDatabase>((resolve, reject) => {
      const request = indexedDb.open(this.databaseName, DATABASE_VERSION);

      request.onerror = () => reject(request.error);
      request.onupgradeneeded = () => {
        const database = request.result;

        for (const name of STORE_NAMES) {
          if (database.objectStoreNames.contains(name)) {
            database.deleteObjectStore(name);
          }

          database.createObjectStore(name, { keyPath: 'key' });
        }
      };
      request.onsuccess = () => {
        const database = request.result;
        database.onversionchange = () => database.close();
        resolve(database);
      };
    });
  }

  private purgeDatabase(): Promise<boolean> {
    const indexedDb = this.indexedDb;

    if (!indexedDb) {
      return Promise.resolve(false);
    }

    return new Promise((resolve) => {
      const request = indexedDb.deleteDatabase(this.databaseName);

      request.onsuccess = () => resolve(true);
      request.onerror = () => resolve(false);
    });
  }
}

export function validatePersistedEnvelope<T>(
  candidate: unknown,
  payloadSchema: ZodType<T>,
): CacheEnvelope<T> | null {
  const parsed = createCacheEnvelopeSchema(payloadSchema).safeParse(candidate);

  return parsed.success ? parsed.data : null;
}

export function createCacheEntryIndexRecord(entry: CacheEnvelope<unknown>): CacheEntryIndexRecord {
  return {
    key: entry.key,
    bytes: entry.bytes,
    lastAccessedAt: entry.lastAccessedAt,
    retentionExpiresAt: entry.retentionExpiresAt,
    tags: entry.tags,
  };
}

export function evictEntries<T extends CacheEntryIndexRecord>(
  entries: readonly T[],
  options: EvictionOptions,
): T[] {
  const retained = [...entries];
  const evicted: T[] = [];

  const ordered = [...retained].sort((left, right) => {
    const expiration = Number(options.expiredKeys.has(right.key)) - Number(options.expiredKeys.has(left.key));

    if (expiration !== 0) {
      return expiration;
    }

    const activity = Number(options.activeKeys.has(left.key)) - Number(options.activeKeys.has(right.key));

    if (activity !== 0) {
      return activity;
    }

    return left.lastAccessedAt - right.lastAccessedAt || left.key.localeCompare(right.key);
  });

  let entryCount = retained.length;
  let byteCount = retained.reduce((total, entry) => total + entry.bytes, 0);

  for (const entry of ordered) {
    const isExpired = options.expiredKeys.has(entry.key);

    if (!isExpired && entryCount <= options.maxEntries && byteCount <= options.maxBytes) {
      break;
    }

    evicted.push(entry);
    entryCount -= 1;
    byteCount -= entry.bytes;
  }

  return evicted;
}

function isStorableEntry(entry: CacheEnvelope<unknown>, limits: CacheLimits): boolean {
  const parsed = validatePersistedEnvelope(entry, z.unknown());
  const limit = entry.key.includes('|k=note-body|') ? limits.maxNoteBodyBytes : limits.maxOtherEntryBytes;

  return parsed !== null && entry.bytes <= limit;
}

function isVersionError(error: unknown): boolean {
  return error instanceof DOMException && error.name === 'VersionError';
}

function validateIndexRecord(candidate: unknown): CacheEntryIndexRecord | null {
  const parsed = cacheEntryIndexRecordSchema.safeParse(candidate);

  return parsed.success ? parsed.data : null;
}

function cacheKeyOf(candidate: unknown): string | null {
  if (!candidate || typeof candidate !== 'object' || !('key' in candidate)) {
    return null;
  }

  const { key } = candidate;

  return typeof key === 'string' ? key : null;
}

function matchesScope(candidate: unknown, scope: CacheDeleteScope): candidate is CacheEntryIndexRecord {
  const record = validateIndexRecord(candidate);

  if (!record?.key.includes(`|p=${scope.principal}|`)) return false;
  if (scope.workspaceId !== undefined && !record.key.includes(`|w=${scope.workspaceId}|`)) return false;
  return scope.tagsAny === undefined || scope.tagsAny.length === 0
    ? true
    : record.tags.some((tag) => scope.tagsAny?.includes(tag));
}

function transactionResult(transaction: IDBTransaction): Promise<boolean> {
  return new Promise((resolve) => {
    transaction.oncomplete = () => resolve(true);
    transaction.onabort = () => resolve(false);
    transaction.onerror = () => resolve(false);
  });
}
