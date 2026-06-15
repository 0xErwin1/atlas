import { defineStore } from 'pinia';
import { ref } from 'vue';

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

  return {
    activeWorkspaceSlug,
    projects,
    setActiveWorkspace,
  };
});
