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
    });
}

export function createCacheEnvelope<T>(input: Omit<CacheEnvelope<T>, 'schema'>): CacheEnvelope<T> {
  return {
    schema: CACHE_SCHEMA_VERSION,
    ...input,
  };
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

function isCanonicalPrincipal(principal: string | null | undefined): principal is string {
  return typeof principal === 'string' && PRINCIPAL_PATTERN.test(principal);
}

function isCanonicalWorkspaceId(workspaceId: string | null | undefined): workspaceId is string {
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
