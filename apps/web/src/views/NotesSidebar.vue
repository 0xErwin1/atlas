<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import NotesTree from '@/components/notas/NotesTree.vue';
import { useDocumentsStore } from '@/stores/documents';
import { useFoldersStore } from '@/stores/folders';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

const route = useRoute();
const router = useRouter();
const workspace = useWorkspaceStore();
const treeRef = ref<InstanceType<typeof NotesTree> | null>(null);
const folders = useFoldersStore();
const documents = useDocumentsStore();
const ui = useUiStore();

const activeSlug = computed(() => {
  const slug = route.params.slug;
  return typeof slug === 'string' && slug.length > 0 ? slug : null;
});

const activeProject = computed(() => workspace.projects[0] ?? null);

const ws = computed(() => workspace.activeWorkspaceSlug ?? '');

async function loadTree(): Promise<void> {
  const wsSlug = workspace.activeWorkspaceSlug;
  if (wsSlug === null) {
    await workspace.loadProjects('');
    return;
  }

  if (workspace.projects.length === 0) {
    await workspace.loadProjects(wsSlug);
  }

  const project = activeProject.value;
  if (project === null) return;

  await Promise.all([folders.load(wsSlug, project.slug), documents.loadSummaries(wsSlug, project.slug)]);
}

function openDoc(slug: string): void {
  void router.push({ name: 'notes', params: { slug } });
}

async function createDoc(title: string, folderId?: string): Promise<void> {
  const project = activeProject.value;
  if (project === null || ws.value === '') return;

  const slug = await documents.create(ws.value, project.slug, title, folderId);
  if (slug !== null) {
    openDoc(slug);
  } else if (documents.error) {
    ui.showBanner(documents.error, 'error');
  }
}

async function renameDoc(slug: string, title: string): Promise<void> {
  const project = activeProject.value;
  if (project === null || ws.value === '') return;

  const ok = await documents.rename(ws.value, project.slug, slug, title);
  if (!ok && documents.error) {
    ui.showBanner(documents.error, 'error');
  }
}

async function removeDoc(slug: string): Promise<void> {
  const project = activeProject.value;
  if (project === null || ws.value === '') return;

  await documents.remove(ws.value, project.slug, slug);
}

async function createFolder(name: string, parentFolderId?: string): Promise<void> {
  const project = activeProject.value;
  if (project === null || ws.value === '') return;

  const ok = await folders.create(ws.value, project.slug, name, parentFolderId);
  if (!ok && folders.error) {
    ui.showBanner(folders.error, 'error');
  }
}

async function renameFolder(folderId: string, name: string): Promise<void> {
  const project = activeProject.value;
  if (project === null || ws.value === '') return;

  const ok = await folders.rename(ws.value, project.slug, folderId, name);
  if (!ok && folders.error) {
    ui.showBanner(folders.error, 'error');
  }
}

async function removeFolder(folderId: string): Promise<void> {
  const project = activeProject.value;
  if (project === null || ws.value === '') return;

  await folders.remove(ws.value, project.slug, folderId);
}

onMounted(loadTree);
watch(() => workspace.activeWorkspaceSlug, loadTree);

function openNewPage(): void {
  treeRef.value?.openNewPage();
}

defineExpose({ openNewPage });
</script>

<template>
  <NotesTree
    v-if="activeProject"
    ref="treeRef"
    :project-name="activeProject.name"
    :folders="folders.folders"
    :docs="documents.summaries"
    :active-slug="activeSlug"
    @select-doc="openDoc"
    @create-doc="createDoc"
    @rename-doc="renameDoc"
    @remove-doc="removeDoc"
    @create-folder="createFolder"
    @rename-folder="renameFolder"
    @remove-folder="removeFolder"
  />
  <p
    v-else
    style="padding: 8px; font-size: var(--fs-sm); color: var(--c-muted);"
  >
    No project selected.
  </p>
</template>
