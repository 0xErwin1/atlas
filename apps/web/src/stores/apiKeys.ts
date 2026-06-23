import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import { collectPaged } from '@/lib/pagination';

export type ApiKeyDto = components['schemas']['ApiKeyDto'];
export type ApiKeyCreated = components['schemas']['ApiKeyCreated'];
export type ApiKeyGrantDto = components['schemas']['ApiKeyGrantDto'];
export type CreateUserApiKeyRequest = components['schemas']['CreateUserApiKeyRequest'];
export type InitialGrantRequest = components['schemas']['InitialGrantRequest'];

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

  async function loadKeys(): Promise<void> {
    loading.value = true;
    error.value = null;

    try {
      const { items, error: e } = await collectPaged<ApiKeyDto>((cursor) =>
        wrappedClient.GET('/v1/api-keys', {
          params: { query: { limit: 200, ...(cursor !== undefined ? { cursor } : {}) } },
        }),
      );

      if (e !== undefined) {
        error.value = hintOf(e, 'Failed to load API keys');
        keys.value = [];
        return;
      }

      keys.value = items.filter((k) => k.revoked_at == null);
    } catch {
      error.value = "Can't reach the server";
      keys.value = [];
    } finally {
      loading.value = false;
    }
  }

  async function createKey(req: CreateUserApiKeyRequest): Promise<ApiKeyCreated | null> {
    error.value = null;

    try {
      const { data, error: e } = await wrappedClient.POST('/v1/api-keys', {
        body: req,
      });

      if (e || !data) {
        error.value = hintOf(e, 'Failed to create API key');
        return null;
      }

      return data;
    } catch {
      error.value = "Can't reach the server";
      return null;
    }
  }

  async function revokeKey(id: string): Promise<boolean> {
    error.value = null;

    try {
      const { error: e } = await wrappedClient.DELETE('/v1/api-keys/{key_id}', {
        params: { path: { key_id: id } },
      });

      if (e) {
        error.value = hintOf(e, 'Failed to revoke API key');
        return false;
      }

      keys.value = keys.value.filter((k) => k.id !== id);
      return true;
    } catch {
      error.value = "Can't reach the server";
      return false;
    }
  }

  async function loadKeyGrants(keyId: string): Promise<ApiKeyGrantDto[] | null> {
    error.value = null;

    try {
      const { data, error: e } = await wrappedClient.GET('/v1/api-keys/{key_id}/grants', {
        params: { path: { key_id: keyId } },
      });

      if (e || !data) {
        error.value = hintOf(e, 'Failed to load key grants');
        return null;
      }

      return data;
    } catch {
      error.value = "Can't reach the server";
      return null;
    }
  }

  async function revokeKeyGrant(keyId: string, grantId: string): Promise<boolean> {
    error.value = null;

    try {
      const { error: e } = await wrappedClient.DELETE('/v1/api-keys/{key_id}/grants/{grant_id}', {
        params: { path: { key_id: keyId, grant_id: grantId } },
      });

      if (e) {
        error.value = hintOf(e, 'Failed to revoke grant');
        return false;
      }

      return true;
    } catch {
      error.value = "Can't reach the server";
      return false;
    }
  }

  return { keys, loading, error, loadKeys, createKey, revokeKey, loadKeyGrants, revokeKeyGrant };
});
