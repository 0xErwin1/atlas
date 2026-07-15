import { defineStore } from 'pinia';
import { readonly, ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import {
  allowResourceCache,
  blockAndPurgeResourceCache,
  setResourceCachePrincipal,
} from '@/cache/cacheRuntime';
import {
  disposeWorkspaceLiveUpdates,
  setWorkspaceLiveUpdatesAuthorizationInvalidator,
} from '@/lib/workspaceLiveUpdates';
import { useWorkspaceStore } from '@/stores/workspace';

export type MeResponse = components['schemas']['MeResponse'];

export interface Problem {
  type: string;
  title: string;
  status: number;
  hint?: string;
  request_id?: string;
}

export interface LoginResult {
  ok: boolean;
  problem?: Problem;
}

export interface ActionResult {
  ok: boolean;
  problem?: Problem;
}

const UNREACHABLE_PROBLEM: NonNullable<LoginResult['problem']> = {
  type: 'urn:atlas:error:unreachable',
  title: "Can't reach the server",
  status: 0,
  hint: 'The Atlas server is not responding. Check it is running and try again.',
};

export const useAuthStore = defineStore('auth', () => {
  const user = ref<MeResponse | null>(null);
  const isAuthenticated = ref(false);
  const apiKeyWarning = ref(false);
  const sessionGeneration = ref(0);
  let fetchGeneration = 0;

  function hydrateUser(data: MeResponse) {
    setResourceCachePrincipal(`${data.principal_type}:${data.id}`);
    allowResourceCache();
    user.value = data;
    isAuthenticated.value = true;
    apiKeyWarning.value = data.principal_type === 'api_key';
  }

  function clearUser(): Promise<boolean> {
    sessionGeneration.value += 1;
    const purge = blockAndPurgeResourceCache();
    setResourceCachePrincipal(undefined);
    disposeWorkspaceLiveUpdates();
    useWorkspaceStore().clearWorkspaceAliases();
    user.value = null;
    isAuthenticated.value = false;
    apiKeyWarning.value = false;
    return purge;
  }

  setWorkspaceLiveUpdatesAuthorizationInvalidator(clearUser);

  async function fetchMe(): Promise<void> {
    const requestGeneration = ++fetchGeneration;
    const requestSessionGeneration = sessionGeneration.value;
    const { data, error } = await wrappedClient.GET('/api/auth/me', {});

    if (requestGeneration !== fetchGeneration || requestSessionGeneration !== sessionGeneration.value) return;

    if (error || !data) {
      await clearUser();
      return;
    }

    hydrateUser(data);
  }

  async function login(credentials: { username: string; password: string }): Promise<LoginResult> {
    try {
      const { data, error } = await wrappedClient.POST('/api/auth/login', {
        body: credentials,
      });

      if (error || !data) {
        return { ok: false, problem: (error as LoginResult['problem']) ?? UNREACHABLE_PROBLEM };
      }

      await fetchMe();
      return { ok: true };
    } catch {
      // fetch throws (not a 4xx/5xx) when the server is unreachable — surface it
      // as a problem so the UI never fails silently.
      return { ok: false, problem: UNREACHABLE_PROBLEM };
    }
  }

  async function logout(): Promise<void> {
    const purged = await clearUser();
    if (!purged) return;

    try {
      await wrappedClient.POST('/api/auth/logout', {});
    } catch {
      // failure is intentional: always clear local state regardless of server response
    }
  }

  async function updateProfile(patch: { email?: string; display_name?: string }): Promise<ActionResult> {
    try {
      const { error } = await wrappedClient.PATCH('/api/users/me', { body: patch });
      if (error) return { ok: false, problem: error as ActionResult['problem'] };

      await fetchMe();
      return { ok: true };
    } catch {
      return { ok: false, problem: UNREACHABLE_PROBLEM };
    }
  }

  async function changePassword(body: {
    current_password: string;
    new_password: string;
  }): Promise<ActionResult> {
    try {
      const { error } = await wrappedClient.POST('/api/auth/change-password', { body });
      if (error) return { ok: false, problem: error as ActionResult['problem'] };

      return { ok: true };
    } catch {
      return { ok: false, problem: UNREACHABLE_PROBLEM };
    }
  }

  return {
    user,
    isAuthenticated,
    apiKeyWarning,
    sessionGeneration: readonly(sessionGeneration),
    clearUser,
    fetchMe,
    login,
    logout,
    updateProfile,
    changePassword,
  };
});
