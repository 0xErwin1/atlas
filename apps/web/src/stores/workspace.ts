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

  return {
    activeWorkspaceSlug,
    projects,
    workspaces,
    setActiveWorkspace,
    loadWorkspaces,
    loadProjects,
  };
});
