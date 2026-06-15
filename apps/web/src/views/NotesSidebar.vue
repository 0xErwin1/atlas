<script setup lang="ts">
import { computed, onMounted, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import NotesTree from '@/components/notas/NotesTree.vue';
import { useDocumentsStore } from '@/stores/documents';
import { useFoldersStore } from '@/stores/folders';
import { useWorkspaceStore } from '@/stores/workspace';

const route = useRoute();
const router = useRouter();
const workspace = useWorkspaceStore();
const folders = useFoldersStore();
const documents = useDocumentsStore();

const activeSlug = computed(() => {
  const slug = route.params.slug;
  return typeof slug === 'string' && slug.length > 0 ? slug : null;
});

const activeProject = computed(() => workspace.projects[0] ?? null);

async function loadTree(): Promise<void> {
  const ws = workspace.activeWorkspaceSlug;
  if (ws === null) {
    await workspace.loadProjects('');
    return;
  }

  if (workspace.projects.length === 0) {
    await workspace.loadProjects(ws);
  }

  const project = activeProject.value;
  if (project === null) return;

  await Promise.all([folders.load(ws, project.slug), documents.loadSummaries(ws, project.slug)]);
}

function openDoc(slug: string): void {
  void router.push({ name: 'notes', params: { slug } });
}

onMounted(loadTree);
watch(() => workspace.activeWorkspaceSlug, loadTree);
</script>

<template>
  <NotesTree
    v-if="activeProject"
    :project-name="activeProject.name"
    :folders="folders.folders"
    :docs="documents.summaries"
    :active-slug="activeSlug"
    @select-doc="openDoc"
  />
  <p
    v-else
    style="padding: 8px; font-size: var(--fs-sm); color: var(--c-muted);"
  >
    No project selected.
  </p>
</template>
