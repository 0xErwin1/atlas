import { ref } from 'vue';
import { IndexedDbCacheStore } from './indexedDbCacheStore';
import {
  isCanonicalPrincipal,
  isCanonicalWorkspaceId,
  ResourceCache,
  type ResourceCacheRequest,
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
  revalidate<T>(request: ResourceCacheRequest<T>): Promise<void>;
  activate<T>(request: ResourceCacheRequest<T>): void;
}

let resourceCache: ResourceCacheRuntime = new ResourceCache({ store: new IndexedDbCacheStore() });
let currentPrincipal: string | undefined;
export const resourceCacheEpoch = ref(0);
export const resourceCacheIsPurging = ref(false);
const globalPurgeFailures = new Set<string>();
const unresolvedAliasBlocks = new Set<string>();

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
  if (changed) resourceCacheEpoch.value += 1;
}

export function getResourceCachePrincipal(): string | undefined {
  return currentPrincipal;
}

export function blockResourceCacheForUnknownAlias(): void {
  if (currentPrincipal !== undefined) unresolvedAliasBlocks.add(currentPrincipal);
  resourceCache.block();
}

export async function purgeResourceCache(): Promise<boolean> {
  resourceCacheIsPurging.value = true;
  resourceCacheEpoch.value += 1;
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

export function configureResourceCacheForTest(cache: Partial<ResourceCacheRuntime>): void {
  if (cache instanceof ResourceCache) {
    resourceCache = cache;
    globalPurgeFailures.clear();
    unresolvedAliasBlocks.clear();
    return;
  }

  resourceCache = { ...resourceCache, ...cache };
}

export { resourceCache };
