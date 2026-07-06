import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import { errorHint } from '@/lib/apiError';
import { collectPaged } from '@/lib/pagination';

export type ApiKeyDto = components['schemas']['ApiKeyDto'];
export type ApiKeyScope = components['schemas']['ApiKeyScope'];
export type ApiKeyCreated = components['schemas']['ApiKeyCreated'];
export type ApiKeyGrantDto = components['schemas']['ApiKeyGrantDto'];
export type CreateUserApiKeyRequest = components['schemas']['CreateUserApiKeyRequest'];
export type InitialGrantRequest = components['schemas']['InitialGrantRequest'];

export const useApiKeysStore = defineStore('apiKeys', () => {
  const keys = ref<ApiKeyDto[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);

  async function loadKeys(): Promise<void> {
    loading.value = true;
    error.value = null;

    try {
      const { items, error: e } = await collectPaged<ApiKeyDto>((cursor) =>
        wrappedClient.GET('/api/api-keys', {
          params: { query: { limit: 200, ...(cursor !== undefined ? { cursor } : {}) } },
        }),
      );

      if (e !== undefined) {
        error.value = errorHint(e, 'Failed to load API keys');
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
      const { data, error: e } = await wrappedClient.POST('/api/api-keys', {
        body: req,
      });

      if (e || !data) {
        error.value = errorHint(e, 'Failed to create API key');
        return null;
      }

      return data;
    } catch {
      error.value = "Can't reach the server";
      return null;
    }
  }

  async function setKeyGlobal(keyId: string, isGlobal: boolean): Promise<boolean> {
    error.value = null;

    try {
      const { data, error: e } = await wrappedClient.PATCH('/api/api-keys/{key_id}', {
        params: { path: { key_id: keyId } },
        body: { is_global: isGlobal },
      });

      if (e || !data) {
        error.value = errorHint(e, 'Failed to update API key');
        return false;
      }

      const existing = keys.value.find((k) => k.id === keyId);
      if (existing !== undefined) {
        existing.is_global = data.is_global;
      }

      return true;
    } catch {
      error.value = "Can't reach the server";
      return false;
    }
  }

  /**
   * Replaces the key's full capability scope set. The PATCH sends the entire
   * selection (not a delta); on success the server-canonical scope list is
   * reflected onto the local key. Mirrors `setKeyGlobal`: sets `error` and
   * returns false on failure, leaving local state unchanged.
   */
  async function setKeyScopes(keyId: string, scopes: ApiKeyScope[]): Promise<boolean> {
    error.value = null;

    try {
      const { data, error: e } = await wrappedClient.PATCH('/api/api-keys/{key_id}', {
        params: { path: { key_id: keyId } },
        body: { scopes },
      });

      if (e || !data) {
        error.value = errorHint(e, 'Failed to update API key');
        return false;
      }

      const existing = keys.value.find((k) => k.id === keyId);
      if (existing !== undefined) {
        existing.scopes = data.scopes;
      }

      return true;
    } catch {
      error.value = "Can't reach the server";
      return false;
    }
  }

  async function revokeKey(id: string): Promise<boolean> {
    error.value = null;

    try {
      const { error: e } = await wrappedClient.DELETE('/api/api-keys/{key_id}', {
        params: { path: { key_id: id } },
      });

      if (e) {
        error.value = errorHint(e, 'Failed to revoke API key');
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
      const { data, error: e } = await wrappedClient.GET('/api/api-keys/{key_id}/grants', {
        params: { path: { key_id: keyId } },
      });

      if (e || !data) {
        error.value = errorHint(e, 'Failed to load key grants');
        return null;
      }

      return data;
    } catch {
      error.value = "Can't reach the server";
      return null;
    }
  }

  /**
   * Grants (or upserts) a workspace-scope role for this key in the given
   * workspace. The grants endpoint is idempotent on the (principal, resource)
   * pair, so the same call both creates and updates a role. The caller reloads
   * the key's grants on success. Sets `error` and returns false on failure.
   */
  async function setKeyWorkspaceRole(keyId: string, slug: string, role: string): Promise<boolean> {
    error.value = null;

    try {
      const { error: e } = await wrappedClient.POST('/api/workspaces/{ws}/grants', {
        params: { path: { ws: slug } },
        body: { principal: { type: 'api_key', id: keyId }, role },
      });

      if (e) {
        error.value = errorHint(e, 'Failed to grant access');
        return false;
      }

      return true;
    } catch {
      error.value = "Can't reach the server";
      return false;
    }
  }

  async function revokeKeyGrant(keyId: string, grantId: string): Promise<boolean> {
    error.value = null;

    try {
      const { error: e } = await wrappedClient.DELETE('/api/api-keys/{key_id}/grants/{grant_id}', {
        params: { path: { key_id: keyId, grant_id: grantId } },
      });

      if (e) {
        error.value = errorHint(e, 'Failed to revoke grant');
        return false;
      }

      return true;
    } catch {
      error.value = "Can't reach the server";
      return false;
    }
  }

  return {
    keys,
    loading,
    error,
    loadKeys,
    createKey,
    setKeyGlobal,
    setKeyScopes,
    revokeKey,
    loadKeyGrants,
    setKeyWorkspaceRole,
    revokeKeyGrant,
  };
});
