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
  visibility: string;
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
  // True while the rail is switching workspace: the active slug is briefly null
  // during the switch, and this flag lets the router guards tell that transient
  // null apart from a cold start so they neither bootstrap from localStorage nor
  // record the restored resource under the wrong workspace.
  const switching = ref(false);
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
  let membersLoadRequest: object | null = null;

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

  function setActiveWorkspace(slug: string | null) {
    if (slug !== activeWorkspaceSlug.value) {
      membersLoadRequest = null;
      members.value = [];
    }
    activeWorkspaceSlug.value = slug;
  }

  function beginSwitch(): void {
    switching.value = true;
  }

  function endSwitch(): void {
    switching.value = false;
  }

  async function loadWorkspaces(): Promise<string | null> {
    const { data, error } = await wrappedClient.GET('/api/workspaces');

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
      setActiveWorkspace(chosen.slug);
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
    setActiveWorkspace(slug);
    projects.value = [];
    persistWorkspace(slug);
  }

  /**
   * Creates a workspace and switches to it. Returns the new slug, or null on
   * failure (with `error` set).
   */
  async function createWorkspace(name: string): Promise<string | null> {
    const { data, error: apiError } = await wrappedClient.POST('/api/workspaces', {
      body: { name },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to create workspace');
      return null;
    }

    const list = await wrappedClient.GET('/api/workspaces');
    if (list.data !== undefined) workspaces.value = list.data;

    switchWorkspace(data.slug);
    return data.slug;
  }

  async function loadProjects(ws: string): Promise<void> {
    const { items, error } = await collectPaged<ProjectDto>((cursor) =>
      wrappedClient.GET('/api/workspaces/{ws}/projects', {
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
      visibility: p.visibility,
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

    const { data, error: apiError } = await wrappedClient.POST('/api/workspaces/{ws}/projects', {
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
    const { error: apiError } = await wrappedClient.PATCH('/api/workspaces/{ws}/projects/{project_slug}', {
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
   * Updates a project's name, task prefix, and/or visibility. Pass only the fields that should
   * change; the server ignores absent fields. Returns true on success and sets
   * `error` (surfacing the API `hint`) on failure — callers show banners for 409
   * duplicate-prefix and 422 bad-format responses.
   */
  async function updateProject(
    ws: string,
    slug: string,
    patch: { name?: string; task_prefix?: string; visibility?: 'private' | 'workspace' | 'public' },
  ): Promise<boolean> {
    const { error: apiError } = await wrappedClient.PATCH('/api/workspaces/{ws}/projects/{project_slug}', {
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
    const { error: apiError } = await wrappedClient.DELETE('/api/workspaces/{ws}/projects/{project_slug}', {
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
    const { data, error: apiError } = await wrappedClient.PATCH('/api/workspaces/{ws}', {
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
   * Re-slugs a workspace via the admin endpoint (root/system-admin only). Reflects
   * the new slug in any cached list, and if the active workspace was re-slugged,
   * repoints the active slug and the persisted choice so navigation keeps working.
   * Returns true on success; sets `error` and returns false otherwise (e.g. 422
   * for an invalid or already-taken slug).
   */
  async function updateWorkspaceSlug(ws: string, slug: string): Promise<boolean> {
    const { data, error: apiError } = await wrappedClient.PATCH('/api/admin/workspaces/{ws}', {
      params: { path: { ws } },
      body: { slug },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to update workspace slug');
      return false;
    }

    const apply = (list: WorkspaceDto[]): WorkspaceDto[] => list.map((w) => (w.slug === ws ? data : w));

    workspaces.value = apply(workspaces.value);
    adminWorkspaces.value = apply(adminWorkspaces.value);

    if (activeWorkspaceSlug.value === ws) {
      activeWorkspaceSlug.value = data.slug;
      persistWorkspace(data.slug);
    }

    return true;
  }

  /**
   * Soft-deletes a workspace via the admin endpoint (root/system-admin only).
   * Drops it from the cached lists; if it was the active workspace, switches to
   * the first remaining one (or clears the selection). Returns true on success;
   * sets `error` and returns false otherwise.
   */
  async function deleteWorkspace(ws: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.DELETE('/api/admin/workspaces/{ws}', {
      params: { path: { ws } },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to delete workspace');
      return false;
    }

    workspaces.value = workspaces.value.filter((w) => w.slug !== ws);
    adminWorkspaces.value = adminWorkspaces.value.filter((w) => w.slug !== ws);

    if (activeWorkspaceSlug.value === ws) {
      const next = workspaces.value[0]?.slug ?? null;
      setActiveWorkspace(next);
      projects.value = [];
      if (next !== null) persistWorkspace(next);
    }

    return true;
  }

  /**
   * Loads every workspace in the system for the root-only admin panel. Clears the
   * list and sets `error` on failure (e.g. a non-root caller gets 403).
   */
  async function loadAdminWorkspaces(): Promise<void> {
    const { data, error: apiError } = await wrappedClient.GET('/api/admin/workspaces');

    if (apiError !== undefined || data === undefined) {
      adminWorkspaces.value = [];
      error.value = errorHint(apiError, 'Failed to load workspaces');
      return;
    }

    adminWorkspaces.value = data;
  }

  /** Loads workspace members (users and agents) for assignee pickers. */
  async function loadMembers(ws: string): Promise<void> {
    const request = {};
    membersLoadRequest = request;

    try {
      const { data, error: apiError } = await wrappedClient.GET('/api/workspaces/{ws}/members', {
        params: { path: { ws } },
      });

      if (membersLoadRequest !== request || activeWorkspaceSlug.value !== ws) return;

      membersLoadRequest = null;
      if (apiError !== undefined || data === undefined) {
        members.value = [];
        error.value = errorHint(apiError, 'Failed to load members');
        return;
      }

      members.value = data;
    } catch (cause) {
      if (membersLoadRequest !== request || activeWorkspaceSlug.value !== ws) return;

      membersLoadRequest = null;
      members.value = [];
      error.value = errorHint(cause, 'Failed to load members');
    }
  }

  /**
   * Changes a workspace member's role (owner/admin/member) and reloads the
   * member list on success. Surfaces the backend `hint` in `error` for the
   * panel to display — the server is authoritative, so a 403 (insufficient
   * privileges) or 409 (`urn:atlas:error:last-owner`) is reported as-is.
   * Returns true on success.
   */
  async function updateMemberRole(ws: string, userId: string, role: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.PATCH('/api/workspaces/{ws}/members/{user_id}', {
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
    const { error: apiError } = await wrappedClient.DELETE('/api/workspaces/{ws}/members/{user_id}', {
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
    const { data, error: apiError } = await wrappedClient.GET('/api/workspaces/{ws}/assignable-users', {
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
    const { error: apiError } = await wrappedClient.POST('/api/workspaces/{ws}/members', {
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
    switching,
    projects,
    workspaces,
    adminWorkspaces,
    members,
    assignableUsers,
    myWorkspaceRole,
    error,
    setActiveWorkspace,
    beginSwitch,
    endSwitch,
    switchWorkspace,
    createWorkspace,
    renameWorkspace,
    updateWorkspaceSlug,
    deleteWorkspace,
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
