import { wrappedClient } from '@/api/wrapper';
import type { PlatformTransport } from './transport';

export function createBrowserPlatformTransport(): PlatformTransport {
  return {
    isDesktop: false,
    login(credentials) {
      return wrappedClient.POST('/api/auth/login', { body: credentials });
    },
    me() {
      return wrappedClient.GET('/api/auth/me', {});
    },
    resume() {
      return wrappedClient.GET('/api/auth/me', {});
    },
    logout() {
      return wrappedClient.POST('/api/auth/logout', {});
    },
    getOrigin() {
      return Promise.resolve({ data: { origin: globalThis.location?.origin ?? '' } });
    },
    setOrigin() {
      return Promise.resolve({ error: 'Server selection is available in Atlas Desktop' });
    },
    createWorkspaceEventSource(workspaceSlug) {
      return new EventSource(`/api/workspaces/${workspaceSlug}/events`);
    },
  };
}
