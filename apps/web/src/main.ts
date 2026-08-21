import { invoke } from '@tauri-apps/api/core';
import { createPinia } from 'pinia';
import { createApp } from 'vue';
import App from './App.vue';
import { setCacheInvalidationHandler, setRequestOutcomeHandler, setUnauthorizedHandler } from './api/wrapper';
import {
  blockResourceCacheForUnknownAlias,
  invalidateResourceCache,
  setWorkspaceAliasResolver,
} from './cache/cacheRuntime';
import { disposeWorkspaceLiveUpdates } from './lib/workspaceLiveUpdates';
import { createBrowserPlatformTransport } from './platform/browser';
import { createDesktopPlatformTransport } from './platform/desktop';
import { installDesktopZoom } from './platform/desktopZoom';
import { createDesktopFetch, setPlatformFetch } from './platform/fetch';
import { getPlatformTransport, type PlatformTransport, setPlatformTransport } from './platform/transport';
import { router } from './router/index';
import { setDesktopSessionActionHandler, useAuthStore } from './stores/auth';
import { useResourceStatusStore } from './stores/resourceStatus';
import { setWorkspaceAliasInvalidationHandler, useWorkspaceStore } from './stores/workspace';
import './theme/index.css';

const app = createApp(App);
export const appPinia = createPinia();
let removePagehideListener: (() => void) | null = null;
let removeTransportListeners: (() => void) | null = null;

export function bootstrapPlatformTransport<T>(factories: {
  isDesktop: () => boolean;
  browser: () => T;
  desktop: () => T;
}): T {
  return factories.isDesktop() ? factories.desktop() : factories.browser();
}

const isDesktop = '__TAURI_INTERNALS__' in window;
setPlatformTransport(
  bootstrapPlatformTransport<PlatformTransport>({
    isDesktop: () => isDesktop,
    browser: createBrowserPlatformTransport,
    desktop: createDesktopPlatformTransport,
  }),
);
if (isDesktop) setPlatformFetch(createDesktopFetch(invoke));

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

/**
 * Ends the signed-in session and sends the user to the login view, keeping the
 * current location as the post-login redirect. Shared by the API 401 path and
 * the desktop host's session-action signal, so losing the session looks the
 * same no matter which side noticed it first.
 */
async function expireSessionAndRedirect(): Promise<void> {
  const auth = useAuthStore(appPinia);
  const currentRoute = router.currentRoute.value;
  if (!auth.isAuthenticated) return;

  await auth.clearUser();

  const redirect =
    currentRoute.name === 'login' || currentRoute.meta.public === true
      ? null
      : { name: 'login', query: { redirect: currentRoute.fullPath } };

  if (redirect !== null) void router.replace(redirect);
}

setUnauthorizedHandler(expireSessionAndRedirect);
setDesktopSessionActionHandler(() => void expireSessionAndRedirect());

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

// Lets live-event invalidation resolve a workspace slug from the loaded catalog
// instead of only from a previous well-formed event, so an event that cannot be
// scoped no longer blocks the cache globally.
setWorkspaceAliasResolver(
  (workspaceSlug) => useWorkspaceStore(appPinia).workspaceIdForSlug(workspaceSlug) ?? undefined,
);

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
if (isDesktop) installDesktopZoom(getPlatformTransport());

export async function mountAfterAuthenticationInitialization(
  initialize = () => useAuthStore(appPinia).initialize(),
  mount = () => app.mount('#app'),
): Promise<void> {
  try {
    await initialize();
  } catch {
    // Initialization failures leave the auth store unauthenticated; mount the login-capable shell.
  }
  mount();
}

void mountAfterAuthenticationInitialization();
