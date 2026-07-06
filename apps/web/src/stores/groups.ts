import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import { errorHint } from '@/lib/apiError';

export type GroupDto = components['schemas']['GroupDto'];
export type GroupMemberDto = components['schemas']['GroupMemberDto'];

/**
 * Groups store: workspace-scoped principal groups. Backs both the share dialog
 * picker (a group can be granted access alongside users and api keys) and the
 * settings Groups panel (create / delete groups and manage their members).
 */
export const useGroupsStore = defineStore('groups', () => {
  const groups = ref<GroupDto[]>([]);
  const members = ref<GroupMemberDto[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);

  async function load(ws: string): Promise<void> {
    loading.value = true;
    error.value = null;

    const { data, error: apiError } = await wrappedClient.GET('/api/workspaces/{ws}/groups', {
      params: { path: { ws } },
    });

    loading.value = false;

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to load groups');
      return;
    }

    groups.value = data;
  }

  async function create(ws: string, name: string): Promise<boolean> {
    error.value = null;

    const { error: apiError } = await wrappedClient.POST('/api/workspaces/{ws}/groups', {
      params: { path: { ws } },
      body: { name },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to create group');
      return false;
    }

    await load(ws);
    return true;
  }

  async function remove(ws: string, groupId: string): Promise<boolean> {
    error.value = null;

    const { error: apiError } = await wrappedClient.DELETE('/api/workspaces/{ws}/groups/{group_id}', {
      params: { path: { ws, group_id: groupId } },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to delete group');
      return false;
    }

    groups.value = groups.value.filter((g) => g.id !== groupId);
    return true;
  }

  async function loadMembers(ws: string, groupId: string): Promise<void> {
    error.value = null;

    const { data, error: apiError } = await wrappedClient.GET(
      '/api/workspaces/{ws}/groups/{group_id}/members',
      { params: { path: { ws, group_id: groupId } } },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to load group members');
      members.value = [];
      return;
    }

    members.value = data;
  }

  async function addMember(ws: string, groupId: string, userId: string): Promise<boolean> {
    error.value = null;

    const { error: apiError } = await wrappedClient.POST('/api/workspaces/{ws}/groups/{group_id}/members', {
      params: { path: { ws, group_id: groupId } },
      body: { user_id: userId },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to add member');
      return false;
    }

    await loadMembers(ws, groupId);
    return true;
  }

  async function removeMember(ws: string, groupId: string, userId: string): Promise<boolean> {
    error.value = null;

    const { error: apiError } = await wrappedClient.DELETE(
      '/api/workspaces/{ws}/groups/{group_id}/members/{user_id}',
      { params: { path: { ws, group_id: groupId, user_id: userId } } },
    );

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to remove member');
      return false;
    }

    members.value = members.value.filter((m) => m.user_id !== userId);
    return true;
  }

  return {
    groups,
    members,
    loading,
    error,
    load,
    create,
    remove,
    loadMembers,
    addMember,
    removeMember,
  };
});
