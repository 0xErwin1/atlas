import { createRouter, createWebHistory } from 'vue-router';
import { routes } from './routes';

export const router = createRouter({
  history: createWebHistory(),
  routes,
});

function redirectTarget(to: { query: Record<string, unknown> }): string {
  const redirect = to.query.redirect;
  return typeof redirect === 'string' && redirect.length > 0 ? redirect : '/n';
}

router.beforeEach(async (to, _from) => {
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

  if (workspace.activeWorkspaceSlug === null) {
    await workspace.loadWorkspaces();
  }

  const { useUiStateStore } = await import('@/stores/uiState');
  const uiState = useUiStateStore();
  if (!uiState.loaded) {
    await uiState.load();
  }

  return true;
});
