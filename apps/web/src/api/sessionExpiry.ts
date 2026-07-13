import type { RouteLocationNormalizedLoaded, RouteLocationRaw } from 'vue-router';

interface AuthSession {
  isAuthenticated: boolean;
  clearUser(): void;
}

type CurrentRoute = Pick<RouteLocationNormalizedLoaded, 'name' | 'fullPath' | 'meta'>;

export function expireSession(auth: AuthSession, currentRoute: CurrentRoute): RouteLocationRaw | null {
  if (!auth.isAuthenticated) return null;

  auth.clearUser();

  if (currentRoute.name === 'login' || currentRoute.meta.public === true) return null;

  return {
    name: 'login',
    query: { redirect: currentRoute.fullPath },
  };
}
