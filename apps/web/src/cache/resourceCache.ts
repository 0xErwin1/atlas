import { type ZodType, z } from 'zod';

export const CACHE_SCHEMA_VERSION = 1;
export const AUTHORIZATION_LEASE_MS = 24 * 60 * 60 * 1000;

export type CacheResourceKind =
  | 'note-tree'
  | 'note-body'
  | 'note-secondary'
  | 'task-board'
  | 'task-list'
  | 'task-detail'
  | 'task-secondary';

export interface CacheKeyInput {
  principal: string | null | undefined;
  workspaceId: string | null | undefined;
  resourceKind: CacheResourceKind;
  resourceId: string;
  query?: Record<string, unknown>;
  setValuedQueryKeys?: readonly string[];
}

export interface CacheEnvelope<T> {
  schema: typeof CACHE_SCHEMA_VERSION;
  key: string;
  payloadVersion: number;
  storedAt: number;
  validatedAt: number;
  lastAccessedAt: number;
  retentionExpiresAt: number;
  bytes: number;
  stale: boolean;
  tags: string[];
  payload: T;
}

export interface CacheLimits {
  maxBytes: number;
  maxEntries: number;
  maxNoteBodyBytes: number;
  maxOtherEntryBytes: number;
}

export interface CachePolicy {
  enabled: boolean;
  authorizationLeaseMs: number;
  hot: {
    maxEntries: number;
  };
  persistent: CacheLimits;
}

export interface CacheClock {
  now(): number;
}

export interface CacheRandom {
  next(): number;
}

export interface CacheNetwork {
  isOnline(): boolean;
}

export interface CacheTimer {
  schedule(delayMs: number, callback: () => void): unknown;
  clear(handle: unknown): void;
}

export interface ResourceCacheStore {
  get<T>(key: string, payloadSchema: ZodType<T>): Promise<CacheEnvelope<T> | null>;
  putMany(entries: readonly CacheEnvelope<unknown>[]): Promise<boolean>;
  deleteMany(keys: readonly string[]): Promise<boolean>;
  deleteScope?(scope: CacheDeleteScope): Promise<boolean>;
  clear(): Promise<boolean>;
}

export interface CacheDeleteScope {
  principal: string;
  workspaceId?: string;
  tagsAny?: readonly string[];
}

export interface CacheCadence {
  freshForMs: number;
  activeForMs: number;
}

export const CACHE_CADENCE: Record<'catalog' | 'primary' | 'secondary', CacheCadence> = {
  catalog: { freshForMs: 30_000, activeForMs: 60_000 },
  primary: { freshForMs: 120_000, activeForMs: 300_000 },
  secondary: { freshForMs: 60_000, activeForMs: 120_000 },
};

export interface ResourceCacheRequest<T> {
  key: string;
  payloadSchema: ZodType<T>;
  tags: string[];
  deriveTags?: (payload: T) => readonly string[];
  freshForMs: number;
  activeForMs?: number;
  retentionForMs: number;
  load(): Promise<T>;
  publish(payload: T): void;
  isCurrent(): boolean;
}

export interface ResourceCacheRevalidationResult {
  fallback?: boolean;
  published: boolean;
  payload?: unknown;
}

export interface ResourceCacheLoad<T> {
  hydration: Promise<T | null>;
  revalidation: Promise<ResourceCacheRevalidationResult>;
  completion: Promise<{ published: boolean; payload?: T }>;
}

export interface ResourceCacheLoader {
  hydrate<T>(
    request: Pick<ResourceCacheRequest<T>, 'key' | 'payloadSchema' | 'publish' | 'isCurrent'>,
  ): Promise<T | null>;
  revalidate<T>(request: ResourceCacheRequest<T>): Promise<ResourceCacheRevalidationResult>;
  activate<T>(request: ResourceCacheRequest<T>): void;
  isAvailable(): boolean;
}

export interface ResourceCacheOptions {
  store: ResourceCacheStore;
  policy?: CachePolicy;
  clock?: CacheClock;
  random?: CacheRandom;
  timer?: CacheTimer;
}

export const DEFAULT_CACHE_POLICY: CachePolicy = {
  enabled: true,
  authorizationLeaseMs: AUTHORIZATION_LEASE_MS,
  hot: {
    maxEntries: 100,
  },
  persistent: {
    maxBytes: 50 * 1024 * 1024,
    maxEntries: 500,
    maxNoteBodyBytes: 2 * 1024 * 1024,
    maxOtherEntryBytes: 4 * 1024 * 1024,
  },
};

export function startHydrationAndRevalidation<T>(
  cache: ResourceCacheLoader,
  request: ResourceCacheRequest<T>,
): ResourceCacheLoad<T> {
  let authoritativeSettled = false;
  let publishedPayload: T | undefined;
  const hydration = cache.hydrate({
    ...request,
    publish: (payload) => {
      if (!authoritativeSettled) request.publish(payload);
    },
  });
  const revalidation = cache.revalidate({
    ...request,
    publish: (payload) => {
      authoritativeSettled = true;
      publishedPayload = payload;
      request.publish(payload);
    },
  });
  const completion = (async () => {
    try {
      const result = await revalidation;
      authoritativeSettled = true;
      let payload =
        result?.payload === undefined ? publishedPayload : request.payloadSchema.parse(result.payload);
      let published = result?.published ?? publishedPayload !== undefined;

      if (payload === undefined && result?.fallback === true && request.isCurrent()) {
        payload = request.payloadSchema.parse(await request.load());
      }

      if (!published && payload !== undefined && request.isCurrent()) {
        request.publish(payload);
        published = true;
      }

      if (request.isCurrent() && cache.isAvailable()) cache.activate(request);
      return { published, ...(payload === undefined ? {} : { payload }) };
    } catch (error) {
      const denied = isAuthoritativeDenial(error);
      authoritativeSettled = denied;
      if (!denied && request.isCurrent() && cache.isAvailable()) {
        cache.activate(request);
      }
      throw error;
    }
  })();

  void hydration.catch(() => undefined);
  void completion.catch(() => undefined);
  return { hydration, revalidation, completion };
}

export class ResourceCache {
  private readonly activeKeys = new Map<
    string,
    {
      freshForMs: number;
      activeForMs: number;
      tags: string[];
      nextAttemptAt: number;
      failures: number;
      attempting: boolean;
      revalidate: () => Promise<void>;
    }
  >();
  private readonly clock: CacheClock;
  private readonly hot = new Map<string, CacheEnvelope<unknown>>();
  private readonly inflight = new Map<string, Promise<ResourceCacheRevalidationResult>>();
  private readonly inflightPublishers = new Map<
    string,
    { isCurrent: () => boolean; publish: (payload: unknown) => void }
  >();
  private readonly policy: CachePolicy;
  private readonly random: CacheRandom;
  private readonly store: ResourceCacheStore;
  private readonly timer: CacheTimer;
  private blocked = false;
  private purgeFailure = false;
  private pendingPurges = 0;
  private scheduler: unknown | null = null;
  private schedulerDueAt: number | null = null;
  private authorizationLeaseExpiresAt = 0;
  private generation = 0;

  constructor(options: ResourceCacheOptions) {
    this.store = options.store;
    this.policy = options.policy ?? DEFAULT_CACHE_POLICY;
    this.clock = options.clock ?? { now: Date.now };
    this.random = options.random ?? { next: Math.random };
    this.timer = options.timer ?? {
      clear: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
      schedule: (delayMs, callback) => setTimeout(callback, delayMs),
    };

    if (!this.policy.enabled) void this.store.clear();
  }

  async hydrate<T>(
    request: Pick<ResourceCacheRequest<T>, 'key' | 'payloadSchema' | 'publish' | 'isCurrent'>,
  ): Promise<T | null> {
    if (!this.policy.enabled || this.isSuspended() || this.authorizationLeaseExpiresAt <= this.clock.now())
      return null;

    const generation = this.generation;
    let entry = this.hot.get(request.key) as CacheEnvelope<T> | undefined;
    if (entry === undefined) {
      try {
        entry = (await this.store.get(request.key, request.payloadSchema)) ?? undefined;
      } catch {
        return null;
      }

      entry = (this.hot.get(request.key) as CacheEnvelope<T> | undefined) ?? entry;
    }

    if (
      !entry ||
      this.isSuspended() ||
      generation !== this.generation ||
      entry.retentionExpiresAt <= this.clock.now() ||
      !request.isCurrent()
    ) {
      return null;
    }

    this.remember(entry);
    request.publish(entry.payload);
    return entry.payload;
  }

  hydrateAndRevalidate<T>(request: ResourceCacheRequest<T>): ResourceCacheLoad<T> {
    return startHydrationAndRevalidation(this, request);
  }

  revalidate<T>(request: ResourceCacheRequest<T>): Promise<ResourceCacheRevalidationResult> {
    if (this.isSuspended()) return Promise.resolve({ fallback: true, published: false });
    this.inflightPublishers.set(request.key, {
      isCurrent: request.isCurrent,
      publish: (payload) => request.publish(request.payloadSchema.parse(payload)),
    });
    const existing = this.inflight.get(request.key);
    if (existing) return existing;

    const generation = this.generation;
    const revalidation = request
      .load()
      .then(async (payload) => {
        // The payload was successfully fetched over HTTP; parse it up front so the
        // discard branches below can still hand it back to the awaiting caller.
        // They only refuse to publish/persist into a stale context — never throw
        // the fetched value away, which would leave the caller with a payloadless
        // "success".
        const parsedPayload = request.payloadSchema.parse(payload);

        if (this.isSuspended() || generation !== this.generation) {
          return { published: false, payload: parsedPayload };
        }

        const publisher = this.inflightPublishers.get(request.key);
        if (publisher?.isCurrent() !== true) return { published: false, payload: parsedPayload };

        const tags = mergeCacheTags(request.tags, request.deriveTags?.(parsedPayload) ?? []);
        if (!isCachePayloadAllowed(parsedPayload)) throw new Error('Cache payload contains excluded data.');
        const now = this.clock.now();
        const entry = createCacheEnvelope({
          key: request.key,
          payloadVersion: 1,
          storedAt: now,
          validatedAt: now,
          lastAccessedAt: now,
          retentionExpiresAt: now + request.retentionForMs,
          bytes: JSON.stringify(payload).length,
          stale: false,
          tags,
          payload: parsedPayload,
        });

        if (this.policy.enabled) {
          this.remember(entry);
          try {
            const persisted = await this.store.putMany([entry]);
            if (!persisted) {
              this.hot.delete(entry.key);

              return { published: false, payload: parsedPayload };
            }
          } catch {
            this.hot.delete(entry.key);
          }

          const currentPublisher = this.inflightPublishers.get(request.key);
          if (
            this.isSuspended() ||
            generation !== this.generation ||
            currentPublisher?.isCurrent() !== true
          ) {
            this.hot.delete(entry.key);
            await this.store.deleteMany([entry.key]);
            return { published: false, payload: parsedPayload };
          }
        }

        if (!this.isSuspended() && generation === this.generation && publisher.isCurrent()) {
          const active = this.activeKeys.get(request.key);
          if (active !== undefined) active.tags = tags;
          publisher.publish(parsedPayload);
          return { published: true, payload: parsedPayload };
        }

        return { published: false, payload: parsedPayload };
      })
      .finally(() => {
        if (this.inflight.get(request.key) === revalidation) {
          this.inflight.delete(request.key);
          this.inflightPublishers.delete(request.key);
        }
      });

    this.inflight.set(request.key, revalidation);
    return revalidation;
  }

  activate<T>(request: ResourceCacheRequest<T>): void;
  activate(key: string, revalidate: () => Promise<void>, freshForMs?: number): void;
  activate<T>(
    requestOrKey: ResourceCacheRequest<T> | string,
    revalidate?: () => Promise<void>,
    freshForMs = 60_000,
  ): void {
    const active =
      typeof requestOrKey === 'string'
        ? { key: requestOrKey, freshForMs, revalidate: revalidate ?? (() => Promise.resolve()) }
        : {
            key: requestOrKey.key,
            freshForMs: requestOrKey.freshForMs,
            revalidate: async () => {
              await this.revalidate(requestOrKey);
            },
          };

    const activeForMs =
      typeof requestOrKey === 'string' ? freshForMs : (requestOrKey.activeForMs ?? requestOrKey.freshForMs);
    if (this.isSuspended()) return;

    const nextAttemptAt = this.clock.now() + activeForMs;
    this.activeKeys.set(active.key, {
      freshForMs: active.freshForMs,
      activeForMs,
      tags:
        typeof requestOrKey === 'string'
          ? []
          : mergeCacheTags(requestOrKey.tags, this.hot.get(requestOrKey.key)?.tags ?? []),
      nextAttemptAt,
      failures: 0,
      attempting: false,
      revalidate: active.revalidate,
    });
    this.rescheduleIfEarlier(nextAttemptAt);
  }

  deactivate(key: string): void {
    this.activeKeys.delete(key);
  }

  isAvailable(): boolean {
    return !this.isSuspended();
  }

  async retry(key: string): Promise<void> {
    if (this.isSuspended()) return;

    const active = this.activeKeys.get(key);
    if (!active) return;

    try {
      await active.revalidate();
      this.recordRevalidationResult(key, active, true);
    } catch {
      this.recordRevalidationResult(key, active, false);
    }

    this.reschedule();
  }

  async purge(): Promise<boolean> {
    const purge = this.beginPurge();
    this.hot.clear();
    this.authorizationLeaseExpiresAt = 0;
    this.dropActiveCallbacks();
    return this.finishPurge(purge, await this.store.clear());
  }

  async purgeWorkspace(workspaceId: string, principal?: string): Promise<boolean> {
    if (principal === undefined) {
      this.block();
      return false;
    }

    const purge = this.beginPurge();

    const keys = [...this.hot.values()]
      .filter((entry) => cacheKeyMatchesScope(entry.key, principal, workspaceId))
      .map((entry) => entry.key);

    for (const key of keys) this.hot.delete(key);
    const hotDeleted = keys.length === 0 || (await this.store.deleteMany(keys));
    const coldDeleted = await (this.store.deleteScope?.({ principal, workspaceId }) ??
      Promise.resolve(false));
    const deleted = hotDeleted && coldDeleted;
    return this.finishPurge(purge, deleted);
  }

  clear(): Promise<boolean> {
    return this.purge();
  }

  block(): void {
    this.generation += 1;
    this.blocked = true;
    this.hot.clear();
    this.authorizationLeaseExpiresAt = 0;
    this.dropActiveCallbacks();
  }

  allow(): void {
    this.blocked = false;
    this.authorizationLeaseExpiresAt = this.clock.now() + this.policy.authorizationLeaseMs;
  }

  async purgeTags(tags: readonly string[], principal?: string, workspaceId?: string): Promise<boolean> {
    if (principal === undefined) {
      this.block();
      return false;
    }

    const tagSet = new Set(tags);
    const purge = this.beginPurge();
    const keys = [...this.hot.values()]
      .filter(
        (entry) =>
          cacheKeyMatchesScope(entry.key, principal, workspaceId) &&
          entry.tags.some((tag) => tagSet.has(tag)),
      )
      .map((entry) => entry.key);

    for (const key of keys) this.hot.delete(key);
    const hotDeleted = keys.length === 0 || (await this.store.deleteMany(keys));
    const coldDeleted = await (this.store.deleteScope?.({ principal, workspaceId, tagsAny: tags }) ??
      Promise.resolve(false));
    const deleted = hotDeleted && coldDeleted;
    return this.finishPurge(purge, deleted);
  }

  dispose(): void {
    this.block();
  }

  private remember(entry: CacheEnvelope<unknown>): void {
    this.hot.delete(entry.key);
    this.hot.set(entry.key, entry);

    while (this.hot.size > this.policy.hot.maxEntries) {
      const oldest = this.hot.keys().next().value;
      if (oldest === undefined) return;
      this.hot.delete(oldest);
    }
  }

  private schedule(): void {
    if (this.scheduler !== null || this.activeKeys.size === 0 || this.isSuspended()) return;

    const delay = this.nextScheduleDelay();
    this.schedulerDueAt = this.clock.now() + delay;
    this.scheduler = this.timer.schedule(delay, () => {
      this.scheduler = null;
      this.schedulerDueAt = null;
      const now = this.clock.now();
      for (const [key, request] of this.activeKeys.entries()) {
        if (request.nextAttemptAt > now) continue;

        const entry = this.hot.get(key);
        if (entry !== undefined && entry.validatedAt + request.freshForMs > now) {
          request.nextAttemptAt = entry.validatedAt + request.freshForMs;
          continue;
        }

        this.runScheduledRevalidation(key, request, now);
      }

      this.schedule();
    });
  }

  private runScheduledRevalidation(
    key: string,
    active: {
      activeForMs: number;
      attempting: boolean;
      failures: number;
      nextAttemptAt: number;
      revalidate: () => Promise<void>;
    },
    now: number,
  ): void {
    if (active.attempting) {
      active.nextAttemptAt = now + active.activeForMs;
      return;
    }

    active.attempting = true;
    active.nextAttemptAt = now + active.activeForMs;

    void active
      .revalidate()
      .then(
        () => this.recordRevalidationResult(key, active, true),
        () => this.recordRevalidationResult(key, active, false),
      )
      .finally(() => {
        if (this.activeKeys.get(key) !== active) return;

        active.attempting = false;
        this.reschedule();
      });
  }

  private dropActiveCallbacks(
    matches:
      | ((
          key: string,
          active: {
            tags: string[];
          },
        ) => boolean)
      | undefined = undefined,
  ): void {
    if (this.scheduler !== null) this.timer.clear(this.scheduler);
    this.scheduler = null;
    this.schedulerDueAt = null;
    if (matches === undefined) {
      this.activeKeys.clear();
      return;
    }

    for (const [key, active] of this.activeKeys) {
      if (matches(key, active)) this.activeKeys.delete(key);
    }
  }

  private recordRevalidationResult(
    key: string,
    active: { activeForMs: number; failures: number; nextAttemptAt: number },
    succeeded: boolean,
    now = this.clock.now(),
  ): void {
    const current = this.activeKeys.get(key);
    if (current !== active) return;

    if (succeeded) {
      current.failures = 0;
      current.nextAttemptAt = now + current.activeForMs;
      return;
    }

    current.failures += 1;
    current.nextAttemptAt = now + jitteredBackoff(current.failures, this.random);
  }

  private reschedule(): void {
    if (this.scheduler !== null) this.timer.clear(this.scheduler);
    this.scheduler = null;
    this.schedulerDueAt = null;
    this.schedule();
  }

  private rescheduleIfEarlier(nextAttemptAt: number): void {
    if (this.scheduler === null || this.schedulerDueAt === null) {
      this.schedule();
      return;
    }

    if (nextAttemptAt < this.schedulerDueAt) this.reschedule();
  }

  private nextScheduleDelay(): number {
    const now = this.clock.now();
    return Math.max(
      0,
      Math.min(...[...this.activeKeys.values()].map((request) => request.nextAttemptAt - now)),
    );
  }

  private beginPurge(): {
    canReleaseFailure: boolean;
  } {
    const canReleaseFailure = this.pendingPurges === 0;
    this.pendingPurges += 1;
    this.generation += 1;
    this.pauseScheduler();
    return { canReleaseFailure };
  }

  private pauseScheduler(): void {
    if (this.scheduler !== null) this.timer.clear(this.scheduler);
    this.scheduler = null;
    this.schedulerDueAt = null;
  }

  private finishPurge(purge: { canReleaseFailure: boolean }, succeeded: boolean): boolean {
    this.pendingPurges -= 1;

    if (!succeeded) {
      this.purgeFailure = true;
      this.block();
    }

    if (this.pendingPurges > 0) return succeeded;

    if (this.purgeFailure) {
      if (succeeded && purge.canReleaseFailure) {
        this.purgeFailure = false;
        this.blocked = false;
      } else {
        this.block();
      }
    }

    if (!this.isSuspended()) this.reschedule();
    return succeeded;
  }

  private isSuspended(): boolean {
    return this.blocked || this.pendingPurges > 0;
  }
}

function isAuthoritativeDenial(error: unknown): boolean {
  const status = (error as { status?: unknown } | null)?.status;
  return status === 403 || status === 404;
}

function jitteredBackoff(failures: number, random: CacheRandom): number {
  const base = [60_000, 120_000, 240_000, 480_000, 900_000][Math.min(failures - 1, 4)] ?? 900_000;
  return Math.round(base * (0.8 + random.next() * 0.4));
}

function cacheKeyMatchesScope(key: string, principal: string, workspaceId?: string): boolean {
  return (
    key.includes(`|p=${principal}|`) && (workspaceId === undefined || key.includes(`|w=${workspaceId}|`))
  );
}

const envelopeShape = {
  schema: z.literal(CACHE_SCHEMA_VERSION),
  key: z.string().min(1),
  payloadVersion: z.number().int().nonnegative(),
  storedAt: z.number().finite().nonnegative(),
  validatedAt: z.number().finite().nonnegative(),
  lastAccessedAt: z.number().finite().nonnegative(),
  retentionExpiresAt: z.number().finite().nonnegative(),
  bytes: z.number().finite().nonnegative(),
  stale: z.boolean(),
  tags: z.array(z.string()),
};

export function buildCacheKey(input: CacheKeyInput): string | null {
  if (
    !isCanonicalPrincipal(input.principal) ||
    !isCanonicalWorkspaceId(input.workspaceId) ||
    !isCanonicalResourceId(input.resourceId)
  ) {
    return null;
  }

  const query = canonicalizeQuery(input.query ?? {}, new Set(input.setValuedQueryKeys));

  return [
    'v1',
    `p=${input.principal}`,
    `w=${input.workspaceId}`,
    `k=${input.resourceKind}`,
    `r=${input.resourceId}`,
    `q=${JSON.stringify(query)}`,
  ].join('|');
}

export function createCacheEnvelopeSchema<T>(payloadSchema: ZodType<T>) {
  return z
    .object({
      ...envelopeShape,
      payload: payloadSchema,
    })
    .superRefine((envelope, context) => {
      if (!isCanonicalCacheKey(envelope.key)) {
        context.addIssue({ code: z.ZodIssueCode.custom, message: 'Cache key is not canonical.' });
      }

      if (!isCachePayloadAllowed(envelope.payload)) {
        context.addIssue({ code: z.ZodIssueCode.custom, message: 'Cache payload contains excluded data.' });
      }

      if (!areCacheTagsValid(envelope.tags)) {
        context.addIssue({ code: z.ZodIssueCode.custom, message: 'Cache tags are invalid.' });
      }
    });
}

export function createCacheEnvelope<T>(input: Omit<CacheEnvelope<T>, 'schema'>): CacheEnvelope<T> {
  return {
    schema: CACHE_SCHEMA_VERSION,
    ...input,
  };
}

function mergeCacheTags(staticTags: readonly string[], derivedTags: readonly string[]): string[] {
  const tags = [...new Set([...staticTags, ...derivedTags])];
  if (!areCacheTagsValid(tags)) throw new Error('Cache tags are invalid.');
  return tags;
}

export function isCacheEnabled(policy: CachePolicy = DEFAULT_CACHE_POLICY): boolean {
  return policy.enabled;
}

function canonicalizeQuery(value: unknown, setValuedKeys: ReadonlySet<string>, key?: string): unknown {
  if (Array.isArray(value)) {
    const items = value.map((item) => canonicalizeQuery(item, setValuedKeys));

    return key && setValuedKeys.has(key)
      ? [...items].sort((left, right) => JSON.stringify(left).localeCompare(JSON.stringify(right)))
      : items;
  }

  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value)
        .filter(([, item]) => item !== undefined)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([entryKey, item]) => [entryKey, canonicalizeQuery(item, setValuedKeys, entryKey)]),
    );
  }

  return value;
}

const UUID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const PRINCIPAL_PATTERN =
  /^(?:user|api_key):[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;
const resourceKinds = new Set<CacheResourceKind>([
  'note-tree',
  'note-body',
  'note-secondary',
  'task-board',
  'task-list',
  'task-detail',
  'task-secondary',
]);
const excludedPayloadKey =
  /(?:authorization|cookie|credential|password|secret|token|api[_-]?key|attachment.*(?:bytes|data|content))/i;
const CACHE_TAG_PATTERN = /^[A-Za-z0-9._:-]+$/;

function areCacheTagsValid(tags: readonly string[]): boolean {
  return tags.every(
    (tag) =>
      tag.length > 0 && tag.length <= 200 && CACHE_TAG_PATTERN.test(tag) && !excludedPayloadKey.test(tag),
  );
}

export function isCanonicalPrincipal(principal: string | null | undefined): principal is string {
  return typeof principal === 'string' && PRINCIPAL_PATTERN.test(principal);
}

export function isCanonicalWorkspaceId(workspaceId: string | null | undefined): workspaceId is string {
  return typeof workspaceId === 'string' && UUID_PATTERN.test(workspaceId);
}

function isCanonicalResourceId(resourceId: string): boolean {
  return resourceId.trim() === resourceId && resourceId.length > 0 && !resourceId.includes('|');
}

function isCanonicalCacheKey(key: string): boolean {
  const parts = key.split('|');

  if (parts.length !== 6 || parts[0] !== 'v1') {
    return false;
  }

  const [principal, workspaceId, resourceKind, resourceId, query] = parts.slice(1);

  if (
    !principal?.startsWith('p=') ||
    !workspaceId?.startsWith('w=') ||
    !resourceKind?.startsWith('k=') ||
    !resourceId?.startsWith('r=') ||
    !query?.startsWith('q=')
  ) {
    return false;
  }

  if (
    !isCanonicalPrincipal(principal.slice(2)) ||
    !isCanonicalWorkspaceId(workspaceId.slice(2)) ||
    !resourceKinds.has(resourceKind.slice(2) as CacheResourceKind) ||
    !isCanonicalResourceId(resourceId.slice(2))
  ) {
    return false;
  }

  try {
    const parsedQuery = JSON.parse(query.slice(2));

    return JSON.stringify(canonicalizeQuery(parsedQuery, new Set())) === query.slice(2);
  } catch {
    return false;
  }
}

function isCachePayloadAllowed(payload: unknown): boolean {
  return !containsExcludedPayload(payload, new Set());
}

function containsExcludedPayload(value: unknown, seen: Set<object>): boolean {
  if (value instanceof ArrayBuffer || ArrayBuffer.isView(value)) {
    return true;
  }

  if (typeof Blob !== 'undefined' && value instanceof Blob) {
    return true;
  }

  if (!value || typeof value !== 'object') {
    return false;
  }

  if (seen.has(value)) {
    return false;
  }

  seen.add(value);

  try {
    return Object.entries(value).some(
      ([key, item]) => excludedPayloadKey.test(key) || containsExcludedPayload(item, seen),
    );
  } catch {
    return true;
  }
}
