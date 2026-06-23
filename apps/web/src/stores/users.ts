import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';

export type UserDto = components['schemas']['UserDto'];

interface ApiProblem {
  title?: string;
  hint?: string;
}

function hintOf(error: unknown, fallback: string): string {
  const p = error as ApiProblem | undefined;
  return p?.hint ?? p?.title ?? fallback;
}

export const useUsersStore = defineStore('users', () => {
  const users = ref<UserDto[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);

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
    password: string;
    email: string | null;
  }): Promise<UserDto | null> {
    error.value = null;

    try {
      const { data, error: e } = await wrappedClient.POST('/v1/users', { body });
      if (e || !data) {
        error.value = hintOf(e, 'Failed to create user');
        return null;
      }
      users.value = [...users.value, data];
      return data;
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

  return { users, loading, error, loadUsers, createUser, setDisabled, resetPassword, setSystemAdmin };
});
