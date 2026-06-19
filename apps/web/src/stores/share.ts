import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import { type GrantRole, isRoleAllowedFor } from '@/lib/grantRoles';
import { collectPaged } from '@/lib/pagination';

export type GrantDto = components['schemas']['GrantDto'];
export type GrantPrincipal = components['schemas']['GrantPrincipal'];
export type PrincipalDto = components['schemas']['PrincipalDto'];

function hintOf(apiError: unknown, fallback: string): string {
  return (apiError as { hint?: string } | undefined)?.hint ?? fallback;
}

/**
 * Share store: the single caller of the workspace grants routes for the share
 * dialog (REQ-W26/W27). Lists/adds/removes grants and changes a principal's
 * role. The agent cap (E03 guard) is enforced here too — a non-user principal
 * can never be sent admin, even if a caller asks for it; the request is refused
 * before the network call.
 */
export const useShareStore = defineStore('share', () => {
  const grants = ref<GrantDto[]>([]);
  const members = ref<PrincipalDto[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);

  async function load(ws: string): Promise<void> {
    loading.value = true;
    error.value = null;

    const { items, error: apiError } = await collectPaged<GrantDto>((cursor) =>
      wrappedClient.GET('/v1/workspaces/{ws}/grants', {
        params: { path: { ws }, query: { limit: 200, ...(cursor !== undefined ? { cursor } : {}) } },
      }),
    );

    loading.value = false;

    if (apiError !== undefined) {
      error.value = hintOf(apiError, 'Failed to load access');
      return;
    }

    grants.value = items;
  }

  async function loadMembers(ws: string): Promise<void> {
    const { data, error: apiError } = await wrappedClient.GET('/v1/workspaces/{ws}/members', {
      params: { path: { ws } },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = hintOf(apiError, 'Failed to load workspace members');
      return;
    }

    members.value = data;
  }

  async function addGrant(ws: string, principal: GrantPrincipal, role: GrantRole): Promise<boolean> {
    if (!isRoleAllowedFor(principal.type, role)) {
      error.value = 'Agents and scripts cannot be granted the Admin role.';
      return false;
    }

    const { error: apiError } = await wrappedClient.POST('/v1/workspaces/{ws}/grants', {
      params: { path: { ws } },
      body: { principal, role },
    });

    if (apiError !== undefined) {
      error.value = hintOf(apiError, 'Failed to grant access');
      return false;
    }

    await load(ws);
    return true;
  }

  async function changeRole(ws: string, grantId: string, role: GrantRole): Promise<boolean> {
    const existing = grants.value.find((g) => g.id === grantId);

    if (existing === undefined) {
      error.value = 'Grant no longer exists.';
      return false;
    }

    return addGrant(ws, existing.principal, role);
  }

  async function removeGrant(ws: string, grantId: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.DELETE('/v1/workspaces/{ws}/grants/{grant_id}', {
      params: { path: { ws, grant_id: grantId } },
    });

    if (apiError !== undefined) {
      error.value = hintOf(apiError, 'Failed to remove access');
      return false;
    }

    await load(ws);
    return true;
  }

  return {
    grants,
    members,
    loading,
    error,
    load,
    loadMembers,
    addGrant,
    changeRole,
    removeGrant,
  };
});
