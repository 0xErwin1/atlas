import { defineStore } from 'pinia';
import { computed, ref } from 'vue';
import type { components } from '@/api/types';
import { wrappedClient } from '@/api/wrapper';
import { errorHint } from '@/lib/apiError';
import { collectPaged } from '@/lib/pagination';
import { useAuthStore } from '@/stores/auth';

type ProjectDto = components['schemas']['Page_ProjectDto']['items'][number];

export type WorkspaceDto = components['schemas']['WorkspaceDto'];
export type PrincipalDto = components['schemas']['PrincipalDto'];
export type UserDto = components['schemas']['UserDto'];

export interface ProjectSummary {
  slug: string;
  name: string;
  task_prefix: string;
  workspace_id: string;
}

const WORKSPACE_STORAGE_KEY = 'atlas:workspace';

function loadStoredWorkspace(): string | null {
  try {
    return localStorage.getItem(WORKSPACE_STORAGE_KEY);
  } catch {
    return null;
  }
}

function persistWorkspace(slug: string): void {
  try {
    localStorage.setItem(WORKSPACE_STORAGE_KEY, slug);
  } catch {
    // ignore storage errors
  }
}

export const useWorkspaceStore = defineStore('workspace', () => {
  const activeWorkspaceSlug = ref<string | null>(null);
  const projects = ref<ProjectSummary[]>([]);
  const workspaces = ref<WorkspaceDto[]>([]);
  // Every workspace in the system, loaded on demand for the root-only admin
  // panel. Kept separate from `workspaces` (the caller's own memberships).
  const adminWorkspaces = ref<WorkspaceDto[]>([]);
  const members = ref<PrincipalDto[]>([]);
  // Users eligible to be added to the active workspace (not already members and
  // not disabled). Populated on demand by the add-member dialog.
  const assignableUsers = ref<UserDto[]>([]);
  const error = ref<string | null>(null);

  const auth = useAuthStore();

  /**
   * The signed-in user's role in the active workspace, derived from the loaded
   * member list (the server does not put the workspace role on the auth user).
   * Null when the user has no membership row here — which is the case for a
   * break-glass global admin who never joined this workspace.
   */
  const myWorkspaceRole = computed<'owner' | 'admin' | 'member' | null>(() => {
    const id = auth.user?.id;
    if (id == null) return null;

    const self = members.value.find((m) => m.id === id);
    const role = self?.role;
    if (role === 'owner' || role === 'admin' || role === 'member') return role;
    return null;
  });

  function setActiveWorkspace(slug: string) {
    activeWorkspaceSlug.value = slug;
  }

  async function loadWorkspaces(): Promise<string | null> {
    const { data, error } = await wrappedClient.GET('/v1/workspaces');

    if (error !== undefined || data === undefined) {
      const hint = (error as { hint?: string } | undefined)?.hint;
      console.error('loadWorkspaces failed', hint ?? error);
      return null;
    }

    workspaces.value = data;

    // Restore the last-used workspace when it still exists, otherwise the first.
    const stored = loadStoredWorkspace();
    const chosen = data.find((w) => w.slug === stored) ?? data[0];
    if (chosen !== undefined) {
      activeWorkspaceSlug.value = chosen.slug;
      return chosen.slug;
    }

    return null;
  }

  /**
   * Switches the active workspace: clears the cached project list so consumers
   * (watching `activeWorkspaceSlug`) reload for the new workspace, and persists
   * the choice so it survives a refresh.
   */
  function switchWorkspace(slug: string): void {
    if (slug === activeWorkspaceSlug.value) return;
    activeWorkspaceSlug.value = slug;
    projects.value = [];
    persistWorkspace(slug);
  }

  /**
   * Creates a workspace and switches to it. Returns the new slug, or null on
   * failure (with `error` set).
   */
  async function createWorkspace(name: string): Promise<string | null> {
    const { data, error: apiError } = await wrappedClient.POST('/v1/workspaces', {
      body: { name },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to create workspace');
      return null;
    }

    const list = await wrappedClient.GET('/v1/workspaces');
    if (list.data !== undefined) workspaces.value = list.data;

    switchWorkspace(data.slug);
    return data.slug;
  }

  async function loadProjects(ws: string): Promise<void> {
    const { items, error } = await collectPaged<ProjectDto>((cursor) =>
      wrappedClient.GET('/v1/workspaces/{ws}/projects', {
        params: { path: { ws }, query: { limit: 200, ...(cursor !== undefined ? { cursor } : {}) } },
      }),
    );

    if (error !== undefined) {
      projects.value = [];
      return;
    }

    projects.value = items.map((p) => ({
      slug: p.slug,
      name: p.name,
      task_prefix: p.task_prefix,
      workspace_id: p.workspace_id,
    }));
  }

  /**
   * Creates a project from a display name, deriving a URL slug and an uppercase
   * task prefix (both required by the API). Returns the new project's slug, or
   * null on failure (with `error` set).
   */
  async function createProject(ws: string, name: string): Promise<string | null> {
    const slug =
      name
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, '-')
        .replace(/(^-|-$)/g, '')
        .slice(0, 40) || 'project';
    const taskPrefix =
      name
        .toUpperCase()
        .replace(/[^A-Z0-9]/g, '')
        .slice(0, 4) || 'PRJ';

    const { data, error: apiError } = await wrappedClient.POST('/v1/workspaces/{ws}/projects', {
      params: { path: { ws } },
      body: { name, slug, task_prefix: taskPrefix, visibility: 'workspace' },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to create project');
      return null;
    }

    await loadProjects(ws);
    return data.slug ?? slug;
  }

  /** Renames a project. Returns true on success; sets `error` and returns false otherwise. */
  async function renameProject(ws: string, slug: string, name: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.PATCH('/v1/workspaces/{ws}/projects/{project_slug}', {
      params: { path: { ws, project_slug: slug } },
      body: { name },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to rename project');
      return false;
    }

    await loadProjects(ws);
    return true;
  }

  /**
   * Updates a project's name and/or task prefix. Pass only the fields that should
   * change; the server ignores absent fields. Returns true on success and sets
   * `error` (surfacing the API `hint`) on failure — callers show banners for 409
   * duplicate-prefix and 422 bad-format responses.
   */
  async function updateProject(
    ws: string,
    slug: string,
    patch: { name?: string; task_prefix?: string },
  ): Promise<boolean> {
    const { error: apiError } = await wrappedClient.PATCH('/v1/workspaces/{ws}/projects/{project_slug}', {
      params: { path: { ws, project_slug: slug } },
      body: patch,
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to update project');
      return false;
    }

    await loadProjects(ws);
    return true;
  }

  /**
   * Deletes a project and everything under it (boards, folders, documents).
   * Returns true on success; sets `error` and returns false otherwise.
   */
  async function deleteProject(ws: string, slug: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.DELETE('/v1/workspaces/{ws}/projects/{project_slug}', {
      params: { path: { ws, project_slug: slug } },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to delete project');
      return false;
    }

    await loadProjects(ws);
    return true;
  }

  /**
   * Renames a workspace's display name (the slug is never re-derived server-side,
   * so existing links stay valid). Reflects the new name in any cached workspace
   * list — the active memberships and, when present, the admin list. Returns true
   * on success; sets `error` and returns false otherwise.
   */
  async function renameWorkspace(ws: string, name: string): Promise<boolean> {
    const { data, error: apiError } = await wrappedClient.PATCH('/v1/workspaces/{ws}', {
      params: { path: { ws } },
      body: { name },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to rename workspace');
      return false;
    }

    const apply = (list: WorkspaceDto[]): WorkspaceDto[] => list.map((w) => (w.slug === ws ? data : w));

    workspaces.value = apply(workspaces.value);
    adminWorkspaces.value = apply(adminWorkspaces.value);
    return true;
  }

  /**
   * Loads every workspace in the system for the root-only admin panel. Clears the
   * list and sets `error` on failure (e.g. a non-root caller gets 403).
   */
  async function loadAdminWorkspaces(): Promise<void> {
    const { data, error: apiError } = await wrappedClient.GET('/v1/admin/workspaces');

    if (apiError !== undefined || data === undefined) {
      adminWorkspaces.value = [];
      error.value = errorHint(apiError, 'Failed to load workspaces');
      return;
    }

    adminWorkspaces.value = data;
  }

  /** Loads workspace members (users and agents) for assignee pickers. */
  async function loadMembers(ws: string): Promise<void> {
    const { data, error: apiError } = await wrappedClient.GET('/v1/workspaces/{ws}/members', {
      params: { path: { ws } },
    });

    if (apiError !== undefined || data === undefined) {
      members.value = [];
      return;
    }

    members.value = data;
  }

  /**
   * Changes a workspace member's role (owner/admin/member) and reloads the
   * member list on success. Surfaces the backend `hint` in `error` for the
   * panel to display — the server is authoritative, so a 403 (insufficient
   * privileges) or 409 (`urn:atlas:error:last-owner`) is reported as-is.
   * Returns true on success.
   */
  async function updateMemberRole(ws: string, userId: string, role: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.PATCH('/v1/workspaces/{ws}/members/{user_id}', {
      params: { path: { ws, user_id: userId } },
      body: { role },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to update member role');
      return false;
    }

    await loadMembers(ws);
    return true;
  }

  /**
   * Removes a member from the workspace and reloads the member list on success.
   * Surfaces the backend `hint` in `error` (e.g. 403 not allowed, 409 last
   * owner). Returns true on success.
   */
  async function removeMember(ws: string, userId: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.DELETE('/v1/workspaces/{ws}/members/{user_id}', {
      params: { path: { ws, user_id: userId } },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to remove member');
      return false;
    }

    await loadMembers(ws);
    return true;
  }

  /**
   * Loads the users that can be added to the workspace (not already members and
   * not disabled) into `assignableUsers`. Clears the list on failure so the
   * add-member picker never shows stale candidates.
   */
  async function loadAssignableUsers(ws: string): Promise<void> {
    const { data, error: apiError } = await wrappedClient.GET('/v1/workspaces/{ws}/assignable-users', {
      params: { path: { ws } },
    });

    if (apiError !== undefined || data === undefined) {
      assignableUsers.value = [];
      return;
    }

    assignableUsers.value = data;
  }

  /**
   * Adds an existing user to the workspace at the given role. Surfaces the
   * backend `hint` in `error` on failure (e.g. 403 admin granting owner, 404
   * user not found, 409 already a member, 422 disabled user). Returns true on
   * success — the caller reloads the member list.
   */
  async function addMember(ws: string, userId: string, role: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.POST('/v1/workspaces/{ws}/members', {
      params: { path: { ws } },
      body: { user_id: userId, role },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to add member');
      return false;
    }

    return true;
  }

  return {
    activeWorkspaceSlug,
    projects,
    workspaces,
    adminWorkspaces,
    members,
    assignableUsers,
    myWorkspaceRole,
    error,
    setActiveWorkspace,
    switchWorkspace,
    createWorkspace,
    renameWorkspace,
    loadWorkspaces,
    loadAdminWorkspaces,
    loadProjects,
    createProject,
    renameProject,
    updateProject,
    deleteProject,
    loadMembers,
    updateMemberRole,
    removeMember,
    loadAssignableUsers,
    addMember,
  };
});
