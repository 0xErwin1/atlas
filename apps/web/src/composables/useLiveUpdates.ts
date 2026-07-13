import { onScopeDispose, type Ref, watch } from 'vue';
import {
  acquireWorkspaceLiveUpdates,
  type WorkspaceLiveUpdate,
  type WorkspaceLiveUpdateHandlers,
  type WorkspaceLiveUpdateSubscription,
} from '@/lib/workspaceLiveUpdates';

export type LiveUpdateEvent = WorkspaceLiveUpdate;
export type LiveUpdateHandlers = WorkspaceLiveUpdateHandlers;

export function useLiveUpdates(wsSlug: Ref<string>, handlers: LiveUpdateHandlers): void {
  let subscription: WorkspaceLiveUpdateSubscription | null = null;

  watch(
    wsSlug,
    (workspaceSlug) => {
      subscription?.release();
      subscription = acquireWorkspaceLiveUpdates(workspaceSlug, handlers);
    },
    { immediate: true },
  );

  onScopeDispose(() => subscription?.release());
}
