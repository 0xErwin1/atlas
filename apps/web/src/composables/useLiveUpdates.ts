import { onScopeDispose, type Ref, watch } from 'vue';
import { getResourceCachePrincipal, resourceCacheEpoch } from '@/cache/cacheRuntime';
import {
  acquireWorkspaceLiveUpdates,
  type WorkspaceLiveUpdate,
  type WorkspaceLiveUpdateHandlers,
  type WorkspaceLiveUpdateSubscription,
} from '@/lib/workspaceLiveUpdates';

export type LiveUpdateEvent = WorkspaceLiveUpdate;
export type LiveUpdateHandlers = WorkspaceLiveUpdateHandlers;

/**
 * Subscribes the calling scope to a workspace's live stream for as long as it
 * lives.
 *
 * The subscription is re-acquired whenever the workspace changes or the
 * resource-cache epoch moves: an epoch bump retires the broker lifetime the
 * subscription was bound to, and a subscriber left on a retired lifetime would
 * silently stop receiving events. No source is opened while no principal is
 * signed in; the next epoch bump (a principal coming back) acquires again.
 */
export function useLiveUpdates(wsSlug: Ref<string>, handlers: LiveUpdateHandlers): void {
  let subscription: WorkspaceLiveUpdateSubscription | null = null;

  watch(
    [wsSlug, resourceCacheEpoch],
    ([workspaceSlug]) => {
      subscription?.release();
      subscription = null;

      if (getResourceCachePrincipal() === undefined) return;

      subscription = acquireWorkspaceLiveUpdates(workspaceSlug, handlers);
    },
    { immediate: true },
  );

  onScopeDispose(() => subscription?.release());
}
