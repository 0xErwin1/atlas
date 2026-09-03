import { wrappedClient } from '@/api/wrapper';
import type { PlatformTransport } from './transport';

export function createBrowserPlatformTransport(): PlatformTransport {
  return {
    isDesktop: false,
    login(credentials) {
      return wrappedClient.POST('/api/v2/custos/auth/login', { body: credentials });
    },
    me() {
      return wrappedClient.GET('/api/v2/custos/auth/me', {});
    },
    resume() {
      return wrappedClient.GET('/api/v2/custos/auth/me', {});
    },
    logout() {
      return wrappedClient.POST('/api/v2/custos/auth/logout', {});
    },
    getOrigin() {
      return Promise.resolve({ data: { origin: globalThis.location?.origin ?? '' } });
    },
    setOrigin() {
      return Promise.resolve({ error: 'Server selection is available in Atlas Desktop' });
    },
    getWindowDecorations() {
      return Promise.resolve({ error: 'Window decorations are available in Atlas Desktop' });
    },
    setWindowDecorations() {
      return Promise.resolve({ error: 'Window decorations are available in Atlas Desktop' });
    },
    getZoom() {
      return Promise.resolve({ error: 'Zoom is available in Atlas Desktop' });
    },
    setZoom() {
      return Promise.resolve({ error: 'Zoom is available in Atlas Desktop' });
    },
    getStartOnLogin() {
      return Promise.resolve({ error: 'Start on login is available in Atlas Desktop' });
    },
    setStartOnLogin() {
      return Promise.resolve({ error: 'Start on login is available in Atlas Desktop' });
    },
    getSystemTray() {
      return Promise.resolve({ error: 'System tray settings are available in Atlas Desktop' });
    },
    setSystemTray() {
      return Promise.resolve({ error: 'System tray settings are available in Atlas Desktop' });
    },
    createWorkspaceEventSource(workspaceSlug) {
      return new EventSource(`/api/v2/acta/workspaces/${workspaceSlug}/events`);
    },
    readClipboardImage() {
      return Promise.resolve(null);
    },
    // The browser saves through an object-URL anchor instead; see `saveDownload`.
    saveDownload() {
      return Promise.resolve({ error: 'downloads are saved by the browser' });
    },
    // With `noopener` the browser returns null even when the tab opened, so the
    // return value cannot distinguish success from a blocked popup.
    openExternal(url) {
      window.open(url, '_blank', 'noopener,noreferrer');
      return Promise.resolve({});
    },
    publicBase() {
      return globalThis.location?.origin ?? '';
    },
  };
}
