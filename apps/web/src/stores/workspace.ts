import { defineStore } from 'pinia';
import { ref } from 'vue';
import { wrappedClient } from '@/api/wrapper';

export interface ProjectSummary {
  slug: string;
  name: string;
  workspace_id: string;
}

export const useWorkspaceStore = defineStore('workspace', () => {
  const activeWorkspaceSlug = ref<string | null>(null);
  const projects = ref<ProjectSummary[]>([]);

  function setActiveWorkspace(slug: string) {
    activeWorkspaceSlug.value = slug;
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
    setActiveWorkspace,
    loadProjects,
  };
});
