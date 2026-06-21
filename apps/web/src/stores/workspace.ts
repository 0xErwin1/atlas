import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types';
import { wrappedClient } from '@/api/wrapper';
import { collectPaged } from '@/lib/pagination';

type ProjectDto = components['schemas']['Page_ProjectDto']['items'][number];

export type WorkspaceDto = components['schemas']['WorkspaceDto'];
export type PrincipalDto = components['schemas']['PrincipalDto'];

export interface ProjectSummary {
  slug: string;
  name: string;
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
  const error = ref<string | null>(null);

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
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to create workspace';
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
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to create project';
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
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to rename project';
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
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to delete project';
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
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to rename workspace';
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
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to load workspaces';
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

  return {
    activeWorkspaceSlug,
    projects,
    workspaces,
    adminWorkspaces,
    members,
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
    deleteProject,
    loadMembers,
  };
});
