import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types';
import { wrappedClient } from '@/api/wrapper';

export type WorkspaceDto = components['schemas']['WorkspaceDto'];

export interface ProjectSummary {
  slug: string;
  name: string;
  workspace_id: string;
}

export const useWorkspaceStore = defineStore('workspace', () => {
  const activeWorkspaceSlug = ref<string | null>(null);
  const projects = ref<ProjectSummary[]>([]);
  const workspaces = ref<WorkspaceDto[]>([]);
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

    const first = data[0];
    if (first !== undefined) {
      activeWorkspaceSlug.value = first.slug;
      return first.slug;
    }

    return null;
  }

  async function loadProjects(ws: string): Promise<void> {
    const { data, error } = await wrappedClient.GET('/v1/workspaces/{ws}/projects', {
      params: { path: { ws } },
    });

    if (error !== undefined || data === undefined) {
      projects.value = [];
      return;
    }

    projects.value = data.items.map((p) => ({
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

  return {
    activeWorkspaceSlug,
    projects,
    workspaces,
    error,
    setActiveWorkspace,
    loadWorkspaces,
    loadProjects,
    createProject,
  };
});
