import { createPinia } from 'pinia';
import { createApp } from 'vue';
import App from './App.vue';
import { setCacheInvalidationHandler, setRequestOutcomeHandler, setUnauthorizedHandler } from './api/wrapper';
import { blockResourceCacheForUnknownAlias, invalidateResourceCache } from './cache/cacheRuntime';
import { disposeWorkspaceLiveUpdates } from './lib/workspaceLiveUpdates';
import { router } from './router/index';
import { useAuthStore } from './stores/auth';
import { useResourceStatusStore } from './stores/resourceStatus';
import { setWorkspaceAliasInvalidationHandler, useWorkspaceStore } from './stores/workspace';
import './theme/index.css';

const app = createApp(App);
export const appPinia = createPinia();
let removePagehideListener: (() => void) | null = null;
let removeTransportListeners: (() => void) | null = null;

export function registerWorkspaceLiveUpdatesPagehide(): () => void {
  if (removePagehideListener !== null) return removePagehideListener;

  const onPagehide = (event: PageTransitionEvent) => {
    if (!event.persisted) disposeWorkspaceLiveUpdates();
  };

  window.addEventListener('pagehide', onPagehide);
  removePagehideListener = () => {
    window.removeEventListener('pagehide', onPagehide);
    removePagehideListener = null;
  };
  return removePagehideListener;
}

app.use(appPinia);
app.use(router);

setUnauthorizedHandler(async () => {
  const auth = useAuthStore(appPinia);
  const currentRoute = router.currentRoute.value;
  if (!auth.isAuthenticated) return;

  await auth.clearUser();

  const redirect =
    currentRoute.name === 'login' || currentRoute.meta.public === true
      ? null
      : { name: 'login', query: { redirect: currentRoute.fullPath } };

  if (redirect !== null) void router.replace(redirect);
});

setCacheInvalidationHandler(async (scope) => {
  if (scope.workspaceSlug === null || scope.scope === 'none') return;

  const workspace = useWorkspaceStore(appPinia);
  const workspaceId = workspace.workspaceIdForSlug(scope.workspaceSlug);
  if (workspaceId === null) {
    const invalidated = await workspace.queueCacheInvalidation(scope);
    if (!invalidated) blockResourceCacheForUnknownAlias();
    return;
  }

  await invalidateResourceCache(scope.scope, workspaceId, scope.tags);
});

setWorkspaceAliasInvalidationHandler((scope, workspaceId) => {
  if (scope.scope === 'none') return Promise.resolve(true);
  return invalidateResourceCache(scope.scope, workspaceId, scope.tags);
});

export function installTransportStatus(): () => void {
  if (removeTransportListeners !== null) return removeTransportListeners;

  const status = useResourceStatusStore(appPinia);
  let onlineHint = navigator.onLine;

  setRequestOutcomeHandler((outcome) => {
    if (outcome === 'start') status.beginRequest('transport', onlineHint);
    if (outcome === 'success') status.recordRequestSuccess('transport', true);
    if (outcome === 'failure') status.recordRequestFailure('transport', onlineHint);
  });

  const onOnline = () => {
    onlineHint = true;
    status.setReconnecting('transport');
  };
  const onOffline = () => {
    onlineHint = false;
  };

  window.addEventListener('online', onOnline);
  window.addEventListener('offline', onOffline);
  removeTransportListeners = () => {
    window.removeEventListener('online', onOnline);
    window.removeEventListener('offline', onOffline);
    setRequestOutcomeHandler(undefined);
    removeTransportListeners = null;
  };
  return removeTransportListeners;
}

registerWorkspaceLiveUpdatesPagehide();
installTransportStatus();
app.mount('#app');
