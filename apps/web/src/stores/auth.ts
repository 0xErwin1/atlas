import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';

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

  function hydrateUser(data: MeResponse) {
    user.value = data;
    isAuthenticated.value = true;
    apiKeyWarning.value = data.principal_type === 'api_key';
  }

  function clearUser() {
    user.value = null;
    isAuthenticated.value = false;
    apiKeyWarning.value = false;
  }

  async function fetchMe(): Promise<void> {
    const { data, error } = await wrappedClient.GET('/v1/auth/me', {});

    if (error || !data) {
      clearUser();
      return;
    }

    hydrateUser(data);
  }

  async function login(credentials: { username: string; password: string }): Promise<LoginResult> {
    try {
      const { data, error } = await wrappedClient.POST('/v1/auth/login', {
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
    try {
      await wrappedClient.POST('/v1/auth/logout', {});
    } catch {
      // failure is intentional: always clear local state regardless of server response
    } finally {
      clearUser();
    }
  }

  async function updateProfile(patch: { email?: string; display_name?: string }): Promise<ActionResult> {
    try {
      const { error } = await wrappedClient.PATCH('/v1/users/me', { body: patch });
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
      const { error } = await wrappedClient.POST('/v1/auth/change-password', { body });
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
    fetchMe,
    login,
    logout,
    updateProfile,
    changePassword,
  };
});
