import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';

export type ApiKeyDto = components['schemas']['ApiKeyDto'];
export type ApiKeyCreated = components['schemas']['ApiKeyCreated'];

interface ApiProblem {
  title?: string;
  hint?: string;
}

function hintOf(error: unknown, fallback: string): string {
  const p = error as ApiProblem | undefined;
  return p?.hint ?? p?.title ?? fallback;
}

export const useApiKeysStore = defineStore('apiKeys', () => {
  const keys = ref<ApiKeyDto[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);

  async function loadKeys(ws: string): Promise<void> {
    loading.value = true;
    error.value = null;

    try {
      const { data, error: e } = await wrappedClient.GET('/v1/workspaces/{ws}/api-keys', {
        params: { path: { ws } },
      });

      if (e || !data) {
        error.value = hintOf(e, 'Failed to load API keys');
        keys.value = [];
        return;
      }

      // Only active keys are shown; revoked ones stay in the audit trail server-side.
      keys.value = data.items.filter((k) => k.revoked_at == null);
    } catch {
      error.value = 'Can’t reach the server';
      keys.value = [];
    } finally {
      loading.value = false;
    }
  }

  async function createKey(
    ws: string,
    name: string,
    expiresAt: string | null,
  ): Promise<ApiKeyCreated | null> {
    error.value = null;

    try {
      const { data, error: e } = await wrappedClient.POST('/v1/workspaces/{ws}/api-keys', {
        params: { path: { ws } },
        body: { name, expires_at: expiresAt },
      });

      if (e || !data) {
        error.value = hintOf(e, 'Failed to create API key');
        return null;
      }

      return data;
    } catch {
      error.value = 'Can’t reach the server';
      return null;
    }
  }

  async function revokeKey(ws: string, id: string): Promise<boolean> {
    error.value = null;

    try {
      const { error: e } = await wrappedClient.POST('/v1/workspaces/{ws}/api-keys/{key_id}/revoke', {
        params: { path: { ws, key_id: id } },
      });

      if (e) {
        error.value = hintOf(e, 'Failed to revoke API key');
        return false;
      }

      keys.value = keys.value.filter((k) => k.id !== id);
      return true;
    } catch {
      error.value = 'Can’t reach the server';
      return false;
    }
  }

  return { keys, loading, error, loadKeys, createKey, revokeKey };
});
