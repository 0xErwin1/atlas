import { ref } from 'vue';
import { type LiveEnvelope, PRESENCE_UPDATED } from '@/lib/eventTypes';
import { mapLiveCacheInvalidation } from './cacheInvalidation';
import { IndexedDbCacheStore } from './indexedDbCacheStore';
import {
  isCanonicalPrincipal,
  isCanonicalWorkspaceId,
  ResourceCache,
  type ResourceCacheLoad,
  type ResourceCacheRequest,
  type ResourceCacheRevalidationResult,
  startHydrationAndRevalidation,
} from './resourceCache';

interface ResourceCacheRuntime {
  allow(): void;
  block(): void;
  clear(): Promise<boolean>;
  purge(): Promise<boolean>;
  purgeTags(tags: readonly string[], principal?: string, workspaceId?: string): Promise<boolean>;
  purgeWorkspace(workspaceId: string, principal?: string): Promise<boolean>;
  hydrate<T>(
    request: Pick<ResourceCacheRequest<T>, 'key' | 'payloadSchema' | 'publish' | 'isCurrent'>,
  ): Promise<T | null>;
  isAvailable(): boolean;
  revalidate<T>(request: ResourceCacheRequest<T>): Promise<ResourceCacheRevalidationResult>;
  readFresh<T>(
    request: Pick<ResourceCacheRequest<T>, 'key' | 'payloadSchema' | 'freshForMs' | 'publish' | 'isCurrent'>,
  ): T | null;
  activate<T>(request: ResourceCacheRequest<T>): void;
  deactivate(key: string): void;
  retry(key: string): Promise<void>;
}

let resourceCache: ResourceCacheRuntime = new ResourceCache({ store: new IndexedDbCacheStore() });
let currentPrincipal: string | undefined;
export const resourceCacheEpoch = ref(0);
export const resourceCacheIsPurging = ref(false);
const globalPurgeFailures = new Set<string>();
const unresolvedAliasBlocks = new Set<string>();
const workspaceAliases = new Map<string, string>();

export function allowResourceCache(): void {
  if (
    currentPrincipal === undefined ||
    globalPurgeFailures.has(currentPrincipal) ||
    unresolvedAliasBlocks.has(currentPrincipal)
  ) {
    return;
  }

  resourceCache.allow();
}

export async function blockAndPurgeResourceCache(): Promise<boolean> {
  resourceCacheIsPurging.value = true;
  resourceCacheEpoch.value += 1;
  workspaceAliases.clear();
  const principal = currentPrincipal;
  try {
    const purged = await resourceCache.purge();

    if (purged) {
      globalPurgeFailures.clear();
      unresolvedAliasBlocks.clear();
    } else if (principal !== undefined) {
      globalPurgeFailures.add(principal);
    }

    return purged;
  } finally {
    resourceCacheIsPurging.value = false;
  }
}

export function hardRefreshResourceCache(workspaceId: string): Promise<boolean> {
  if (!isCanonicalPrincipal(currentPrincipal) || !isCanonicalWorkspaceId(workspaceId)) {
    return Promise.resolve(false);
  }

  return resourceCache.purgeWorkspace(workspaceId, currentPrincipal);
}

export function setResourceCachePrincipal(principal: string | undefined): void {
  const changed = currentPrincipal !== principal;
  currentPrincipal = principal;
  if (changed) {
    resourceCacheEpoch.value += 1;
    workspaceAliases.clear();
  }
}

export function getResourceCachePrincipal(): string | undefined {
  return currentPrincipal;
}

export function hydrateAndRevalidateResource<T>(request: ResourceCacheRequest<T>): ResourceCacheLoad<T> {
  const epoch = resourceCacheEpoch.value;
  return startHydrationAndRevalidation(resourceCache, {
    ...request,
    isCurrent: () => epoch === resourceCacheEpoch.value && request.isCurrent(),
  });
}

export function blockResourceCacheForUnknownAlias(): void {
  if (currentPrincipal !== undefined) unresolvedAliasBlocks.add(currentPrincipal);
  resourceCache.block();
}

export async function purgeResourceCache(): Promise<boolean> {
  resourceCacheIsPurging.value = true;
  resourceCacheEpoch.value += 1;
  workspaceAliases.clear();
  try {
    return await resourceCache.clear();
  } finally {
    resourceCacheIsPurging.value = false;
  }
}

export async function runHardRefresh(workspaceId: string, reload: () => Promise<unknown>): Promise<boolean> {
  const purged = await hardRefreshResourceCache(workspaceId);
  if (!purged) return false;
  await reload();
  return true;
}

export async function invalidateResourceCache(
  scope: 'workspace' | 'resource',
  workspaceId: string,
  tags: readonly string[],
): Promise<boolean> {
  if (!isCanonicalPrincipal(currentPrincipal) || !isCanonicalWorkspaceId(workspaceId)) return false;
  const invalidated =
    scope === 'workspace'
      ? await resourceCache.purgeWorkspace(workspaceId, currentPrincipal)
      : tags.length === 0
        ? true
        : await resourceCache.purgeTags(tags, currentPrincipal, workspaceId);

  if (invalidated) {
    unresolvedAliasBlocks.delete(currentPrincipal);
    allowResourceCache();
  }

  return invalidated;
}

/**
 * Invalidates cache entries from an SSE envelope, blocking globally when the
 * event cannot be safely scoped to an authoritative workspace UUID.
 */
function resolveWorkspaceAlias(workspaceSlug: string | undefined): string | undefined {
  if (workspaceSlug === undefined) return undefined;

  const workspaceId = workspaceAliases.get(workspaceSlug);
  return isCanonicalWorkspaceId(workspaceId) ? workspaceId : undefined;
}

export async function invalidateLiveResourceCache(
  envelope?: LiveEnvelope,
  workspaceSlug?: string,
): Promise<boolean> {
  if (envelope === undefined) {
    const workspaceId = resolveWorkspaceAlias(workspaceSlug);
    if (workspaceId === undefined) {
      blockResourceCacheForUnknownAlias();
      return false;
    }

    return invalidateResourceCache('workspace', workspaceId, []);
  }

  const invalidation = mapLiveCacheInvalidation(envelope);
  if (invalidation === null) {
    if (envelope.event_type === PRESENCE_UPDATED) return true;

    const workspaceId = resolveWorkspaceAlias(workspaceSlug);
    if (workspaceId === undefined) {
      blockResourceCacheForUnknownAlias();
      return false;
    }

    return invalidateResourceCache('workspace', workspaceId, []);
  }

  if (workspaceSlug !== undefined) {
    const brokerWorkspaceId = resolveWorkspaceAlias(workspaceSlug);
    if (brokerWorkspaceId !== undefined && brokerWorkspaceId !== invalidation.workspaceId) {
      return invalidateResourceCache('workspace', brokerWorkspaceId, []);
    }

    workspaceAliases.set(workspaceSlug, invalidation.workspaceId);
  }

  return invalidateResourceCache(invalidation.scope, invalidation.workspaceId, invalidation.tags ?? []);
}

export async function invalidateWorkspaceTaskQueryCache(workspaceId: string): Promise<boolean> {
  if (!isCanonicalPrincipal(currentPrincipal) || !isCanonicalWorkspaceId(workspaceId)) return false;

  return resourceCache.purgeTags(['workspace-tasks'], currentPrincipal, workspaceId);
}

export async function invalidateTaskCache(
  workspaceId: string,
  readableId: string,
  boardId?: string,
  taskUuid?: string,
): Promise<boolean> {
  if (!isCanonicalPrincipal(currentPrincipal) || !isCanonicalWorkspaceId(workspaceId)) return false;

  const tags = [`task:${readableId}`, 'task-board', 'workspace-tasks'];
  if (boardId !== undefined) tags.push(`board:${boardId}`);
  if (taskUuid !== undefined) tags.push(`task-uuid:${taskUuid}`);

  return resourceCache.purgeTags(tags, currentPrincipal, workspaceId);
}

export function configureResourceCacheForTest(cache: Partial<ResourceCacheRuntime>): void {
  if (cache instanceof ResourceCache) {
    resourceCache = cache;
    globalPurgeFailures.clear();
    unresolvedAliasBlocks.clear();
    workspaceAliases.clear();
    return;
  }

  resourceCache = { ...resourceCache, ...cache };
}

export { resourceCache };
