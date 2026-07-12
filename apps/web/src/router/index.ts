import { createRouter, createWebHistory } from 'vue-router';
import { useLastViewedStore } from '@/stores/lastViewed';
import { useWorkspaceStore } from '@/stores/workspace';
import { routes } from './routes';

export const router = createRouter({
  history: createWebHistory(),
  routes,
});

// Resource-carrying routes mapped to the param that identifies the resource.
// A route counts as "viewed" only when this param is present; a bare section
// root carries no resource and is the empty fallback.
const RESOURCE_PARAM_BY_ROUTE: Record<string, string> = {
  notes: 'slug',
  'task-detail': 'readableId',
  tasks: 'boardId',
  'task-view': 'viewId',
};

function redirectTarget(to: { query: Record<string, unknown> }): string {
  const redirect = to.query.redirect;
  return typeof redirect === 'string' && redirect.length > 0 ? redirect : '/n';
}

export async function workspaceBeforeEach(
  to: import('vue-router').RouteLocationNormalized,
): Promise<boolean | string | { name: string; query: Record<string, string> }> {
  // Public routes (e.g. the activation page) render without auth and never
  // bounce to login — the invitee has no session yet.
  if (to.meta.public === true) {
    return true;
  }

  const { useAuthStore } = await import('@/stores/auth');
  const auth = useAuthStore();

  if (to.name === 'login') {
    // An authenticated visit to /login bounces to the redirect target or default
    // instead of showing the sign-in form again.
    if (!auth.isAuthenticated) await auth.fetchMe();
    return auth.isAuthenticated ? redirectTarget(to) : true;
  }

  if (!auth.isAuthenticated) {
    await auth.fetchMe();
  }

  if (!auth.isAuthenticated) {
    return { name: 'login', query: { redirect: to.fullPath } };
  }

  const { useWorkspaceStore } = await import('@/stores/workspace');
  const workspace = useWorkspaceStore();

  if (workspace.activeWorkspaceSlug === null && !workspace.switching) {
    await workspace.loadWorkspaces();
  }

  const { useUiStateStore } = await import('@/stores/uiState');
  const uiState = useUiStateStore();
  if (!uiState.loaded) {
    await uiState.load();
  }

  return true;
}

/**
 * Central choke point that records the last resource the user viewed per
 * workspace, so a later switch back into that workspace restores it. Runs after
 * navigation resolves, when pinia is active. Skipped mid-switch, when the active
 * workspace has been cleared, so a paramless root never clobbers real history.
 */
export function recordLastViewed(
  to: Pick<import('vue-router').RouteLocationNormalized, 'name' | 'params'>,
): void {
  const routeName = typeof to.name === 'string' ? to.name : '';
  const paramName = RESOURCE_PARAM_BY_ROUTE[routeName];
  if (paramName === undefined) return;

  const value = to.params[paramName];
  if (typeof value !== 'string' || value === '') return;

  const workspace = useWorkspaceStore();
  if (workspace.switching) return;

  const activeSlug = workspace.activeWorkspaceSlug;
  if (activeSlug === null || activeSlug === '') return;

  const lastViewed = useLastViewedStore();
  lastViewed.record(activeSlug, { name: routeName, params: { [paramName]: value } });
}

router.beforeEach(workspaceBeforeEach);
router.afterEach(recordLastViewed);
