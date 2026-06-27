import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import { errorHint } from '@/lib/apiError';
import { type GrantRole, isRoleAllowedFor } from '@/lib/grantRoles';
import { collectPaged } from '@/lib/pagination';

export type GrantDto = components['schemas']['GrantDto'];
export type GrantPrincipal = components['schemas']['GrantPrincipal'];
export type PrincipalDto = components['schemas']['PrincipalDto'];

export type ShareResource =
  | { kind: 'workspace'; ws: string }
  | { kind: 'project'; ws: string; projectSlug: string };

/**
 * Share store: manages grants for both workspace and project resources.
 * The resource descriptor (ShareResource) determines which endpoints are
 * called. The agent cap (E03 guard) is enforced here — a non-user principal
 * can never be sent admin, even if a caller asks for it.
 */
export const useShareStore = defineStore('share', () => {
  const grants = ref<GrantDto[]>([]);
  const members = ref<PrincipalDto[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);

  async function load(resource: ShareResource): Promise<void> {
    loading.value = true;
    error.value = null;

    let items: GrantDto[];
    let apiError: unknown;

    if (resource.kind === 'workspace') {
      const result = await collectPaged<GrantDto>((cursor) =>
        wrappedClient.GET('/v1/workspaces/{ws}/grants', {
          params: {
            path: { ws: resource.ws },
            query: { limit: 200, ...(cursor !== undefined ? { cursor } : {}) },
          },
        }),
      );
      items = result.items;
      apiError = result.error;
    } else {
      const result = await collectPaged<GrantDto>((cursor) =>
        wrappedClient.GET('/v1/workspaces/{ws}/projects/{project_slug}/grants', {
          params: {
            path: { ws: resource.ws, project_slug: resource.projectSlug },
            query: { limit: 200, ...(cursor !== undefined ? { cursor } : {}) },
          },
        }),
      );
      items = result.items;
      apiError = result.error;
    }

    loading.value = false;

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to load access');
      return;
    }

    grants.value = items;
  }

  async function loadMembers(ws: string): Promise<void> {
    const { data, error: apiError } = await wrappedClient.GET('/v1/workspaces/{ws}/members', {
      params: { path: { ws } },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to load workspace members');
      return;
    }

    members.value = data;
  }

  async function addGrant(
    resource: ShareResource,
    principal: GrantPrincipal,
    role: GrantRole,
  ): Promise<boolean> {
    if (!isRoleAllowedFor(principal.type, role)) {
      error.value = 'Agents and scripts cannot be granted the Admin role.';
      return false;
    }

    let apiError: unknown;

    if (resource.kind === 'workspace') {
      const result = await wrappedClient.POST('/v1/workspaces/{ws}/grants', {
        params: { path: { ws: resource.ws } },
        body: { principal, role },
      });
      apiError = result.error;
    } else {
      const result = await wrappedClient.POST('/v1/workspaces/{ws}/projects/{project_slug}/grants', {
        params: { path: { ws: resource.ws, project_slug: resource.projectSlug } },
        body: { principal, role },
      });
      apiError = result.error;
    }

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to grant access');
      return false;
    }

    await load(resource);
    return true;
  }

  async function changeRole(resource: ShareResource, grantId: string, role: GrantRole): Promise<boolean> {
    const existing = grants.value.find((g) => g.id === grantId);

    if (existing === undefined) {
      error.value = 'Grant no longer exists.';
      return false;
    }

    return addGrant(resource, existing.principal, role);
  }

  async function removeGrant(resource: ShareResource, grantId: string): Promise<boolean> {
    let apiError: unknown;

    if (resource.kind === 'workspace') {
      const result = await wrappedClient.DELETE('/v1/workspaces/{ws}/grants/{grant_id}', {
        params: { path: { ws: resource.ws, grant_id: grantId } },
      });
      apiError = result.error;
    } else {
      const result = await wrappedClient.DELETE(
        '/v1/workspaces/{ws}/projects/{project_slug}/grants/{grant_id}',
        {
          params: { path: { ws: resource.ws, project_slug: resource.projectSlug, grant_id: grantId } },
        },
      );
      apiError = result.error;
    }

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to remove access');
      return false;
    }

    await load(resource);
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
