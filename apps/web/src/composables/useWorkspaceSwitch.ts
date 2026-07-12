import { isNavigationFailure, NavigationFailureType, useRoute, useRouter } from 'vue-router';
import { useLastViewedStore } from '@/stores/lastViewed';
import { useWorkspaceStore } from '@/stores/workspace';

// Task surfaces that should keep the user in Tasks after a workspace switch.
const TASK_ROUTES = new Set(['tasks', 'task-view', 'task-detail']);

/**
 * Shared workspace switch-and-restore flow used by the rail switcher: it restores
 * the destination's last-viewed resource instead of duplicating that logic at each
 * call site.
 */
export function useWorkspaceSwitch() {
  const route = useRoute();
  const router = useRouter();
  const workspace = useWorkspaceStore();
  const lastViewed = useLastViewedStore();

  // Keep the user in the same section when they switch workspace: from any Tasks
  // route land on Tasks, from Search stay in Search, otherwise (Notes, Settings,
  // …) go to Notes. The specific resource id belongs to the old workspace, so it
  // is dropped in favour of the section root when the destination has no history.
  function sectionAfterSwitch(): string {
    const name = typeof route.name === 'string' ? route.name : '';
    if (TASK_ROUTES.has(name)) return 'tasks';
    if (name === 'search') return 'search';
    return 'notes';
  }

  /**
   * Switches the active workspace, restoring the resource last viewed in the
   * destination (falling back to the current section's root). The active slug is
   * briefly nulled during navigation so the router guards can tell the transient
   * switch apart from a cold start.
   */
  async function switchTo(slug: string): Promise<void> {
    if (slug === workspace.activeWorkspaceSlug) return;

    const restored = lastViewed.forWorkspace(slug);
    const target = restored ?? { name: sectionAfterSwitch() };

    const token = workspace.beginSwitch();
    workspace.setActiveWorkspace(null);

    try {
      const failure = await router.push(target);

      // A `duplicated` failure (the restored URL equals the current one) is not a
      // real block — the destination is already shown — so we still activate the
      // new workspace. Only an aborted/cancelled navigation reverts the switch.
      const blocked =
        isNavigationFailure(failure, NavigationFailureType.aborted) ||
        isNavigationFailure(failure, NavigationFailureType.cancelled);
      if (blocked) {
        // A superseded switch must not revert: the newer switch owns the outcome.
        // Revert to the last committed workspace, not a per-call snapshot: an
        // overlapping switch may have already nulled the active slug, so the
        // snapshot could be null even though a real workspace is committed.
        if (workspace.isCurrentSwitch(token)) workspace.setActiveWorkspace(workspace.committedSlug);
        return;
      }
    } catch {
      if (workspace.isCurrentSwitch(token)) workspace.setActiveWorkspace(workspace.committedSlug);
      return;
    } finally {
      workspace.endSwitch(token);
    }

    // A superseded switch must not commit its stale target either.
    if (workspace.isCurrentSwitch(token)) workspace.switchWorkspace(slug);
  }

  return { switchTo };
}
