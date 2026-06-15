import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';

export type MeResponse = components['schemas']['MeResponse'];

export interface LoginResult {
  ok: boolean;
  problem?: { type: string; title: string; status: number; hint?: string; request_id?: string };
}

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
    const { data, error } = await wrappedClient.POST('/v1/auth/login', {
      body: credentials,
    });

    if (error || !data) {
      return { ok: false, problem: error as LoginResult['problem'] };
    }

    await fetchMe();
    return { ok: true };
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

  return {
    user,
    isAuthenticated,
    apiKeyWarning,
    fetchMe,
    login,
    logout,
  };
});
