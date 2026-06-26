import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';

export type UserDto = components['schemas']['UserDto'];
export type CreateUserResponse = components['schemas']['CreateUserResponse'];
export type UserMembershipDto = components['schemas']['UserMembershipDto'];

/** A user's workspace memberships as a `slug -> role` lookup. */
export type MembershipMap = Record<string, string>;

interface ApiProblem {
  title?: string;
  hint?: string;
}

function hintOf(error: unknown, fallback: string): string {
  const p = error as ApiProblem | undefined;
  return p?.hint ?? p?.title ?? fallback;
}

/**
 * Turns the bare single-use activation path returned by the API
 * (`/activate/<token>`) into a full URL on the current origin so it can be
 * shared with the invitee as-is.
 */
export function activationUrl(path: string): string {
  if (/^https?:\/\//i.test(path)) return path;
  return `${window.location.origin}${path.startsWith('/') ? '' : '/'}${path}`;
}

export const useUsersStore = defineStore('users', () => {
  const users = ref<UserDto[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);
  // Per-user workspace memberships, keyed by user id, each a `slug -> role`
  // lookup. Populated on demand when a user row is expanded.
  const memberships = ref<Record<string, MembershipMap>>({});

  async function loadUsers(): Promise<void> {
    loading.value = true;
    error.value = null;

    try {
      const { data, error: e } = await wrappedClient.GET('/v1/users', {});
      if (e || !data) {
        error.value = hintOf(e, 'Failed to load users');
        users.value = [];
        return;
      }
      users.value = data;
    } catch {
      error.value = "Can't reach the server";
      users.value = [];
    } finally {
      loading.value = false;
    }
  }

  async function createUser(body: {
    username: string;
    display_name: string;
    email: string | null;
    workspace: string;
    role: string;
  }): Promise<CreateUserResponse | null> {
    error.value = null;

    try {
      const { data, error: e } = await wrappedClient.POST('/v1/users', { body });
      if (e || !data) {
        error.value = hintOf(e, 'Failed to create user');
        return null;
      }
      users.value = [...users.value, data.user];
      return data;
    } catch {
      error.value = "Can't reach the server";
      return null;
    }
  }

  /**
   * Issues a fresh one-time activation link for a pending user, invalidating any
   * prior link. Returns the new link path, or null on failure (`error` is set —
   * a 409 means the user already activated).
   */
  async function regenerateActivationLink(id: string): Promise<string | null> {
    error.value = null;

    try {
      const { data, error: e } = await wrappedClient.POST('/v1/users/{user_id}/activation-link', {
        params: { path: { user_id: id } },
      });
      if (e || !data) {
        error.value = hintOf(e, 'Failed to regenerate activation link');
        return null;
      }
      return data.activation_link;
    } catch {
      error.value = "Can't reach the server";
      return null;
    }
  }

  async function setDisabled(id: string, disabled: boolean): Promise<boolean> {
    error.value = null;

    const path = disabled ? '/v1/users/{user_id}/disable' : '/v1/users/{user_id}/enable';
    try {
      const { error: e } = await wrappedClient.POST(path, { params: { path: { user_id: id } } });
      if (e) {
        error.value = hintOf(e, 'Failed to update user');
        return false;
      }
      await loadUsers();
      return true;
    } catch {
      error.value = "Can't reach the server";
      return false;
    }
  }

  async function resetPassword(id: string, newPassword: string): Promise<boolean> {
    error.value = null;

    try {
      const { error: e } = await wrappedClient.POST('/v1/users/{user_id}/reset-password', {
        params: { path: { user_id: id } },
        body: { new_password: newPassword },
      });
      if (e) {
        error.value = hintOf(e, 'Failed to reset password');
        return false;
      }
      return true;
    } catch {
      error.value = "Can't reach the server";
      return false;
    }
  }

  /**
   * Loads a user's workspace memberships and caches them under `memberships[id]`
   * as a `slug -> role` map for the workspace-access editor. Returns the map, or
   * null on failure (with `error` set).
   */
  async function loadMemberships(id: string): Promise<MembershipMap | null> {
    error.value = null;

    try {
      const { data, error: e } = await wrappedClient.GET('/v1/users/{user_id}/memberships', {
        params: { path: { user_id: id } },
      });
      if (e || !data) {
        error.value = hintOf(e, 'Failed to load memberships');
        return null;
      }

      const map: MembershipMap = {};
      for (const m of data) map[m.workspace_slug] = m.role;

      memberships.value = { ...memberships.value, [id]: map };
      return map;
    } catch {
      error.value = "Can't reach the server";
      return null;
    }
  }

  async function setSystemAdmin(id: string, value: boolean): Promise<UserDto | null> {
    error.value = null;

    try {
      const { data, error: e } = await wrappedClient.POST('/v1/users/{user_id}/system-admin', {
        params: { path: { user_id: id } },
        body: { is_system_admin: value },
      });
      if (e || !data) {
        error.value = hintOf(e, 'Failed to update system-admin status');
        return null;
      }
      users.value = users.value.map((u) => (u.id === id ? data : u));
      return data;
    } catch {
      error.value = "Can't reach the server";
      return null;
    }
  }

  return {
    users,
    loading,
    error,
    memberships,
    loadUsers,
    createUser,
    regenerateActivationLink,
    setDisabled,
    resetPassword,
    setSystemAdmin,
    loadMemberships,
  };
});
