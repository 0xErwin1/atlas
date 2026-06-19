<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import FolderPickerDialog from '@/components/notas/FolderPickerDialog.vue';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import NotesTree from '@/components/notas/NotesTree.vue';
import Dropdown, { type DropdownOption } from '@/components/ui/Dropdown.vue';
import { docKey, type TreeNodeRef } from '@/lib/notesTree';
import { useDocumentsStore } from '@/stores/documents';
import { useFoldersStore } from '@/stores/folders';
import { useTreeSelection } from '@/stores/treeSelection';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

const PROJECT_STORAGE_KEY = 'atlas:notes-project';

function loadStoredProject(): string | null {
  try {
    return localStorage.getItem(PROJECT_STORAGE_KEY);
  } catch {
    return null;
  }
}

const route = useRoute();
const router = useRouter();
const workspace = useWorkspaceStore();
const treeRef = ref<InstanceType<typeof NotesTree> | null>(null);
const folders = useFoldersStore();
const documents = useDocumentsStore();
const selection = useTreeSelection();
const ui = useUiStore();

const activeSlug = computed(() => {
  const slug = route.params.slug;
  return typeof slug === 'string' && slug.length > 0 ? slug : null;
});

// Keep the tree's persistent selection in step with the open document: the
// selection store outlives this view (Pinia), so without this a doc selected
// before switching apps would stay highlighted on return even with nothing open.
// Multi-select gestures (shift/ctrl) do not change the route, so they survive.
watch(
  activeSlug,
  (slug) => {
    if (slug === null) selection.clear();
    else selection.selectOnly(docKey(slug));
  },
  { immediate: true },
);

// Which project's tree the Notes view shows. Persisted so the choice sticks
// across sessions; falls back to the first project when unset or stale.
const selectedSlug = ref<string | null>(loadStoredProject());

const activeProject = computed(
  () => workspace.projects.find((p) => p.slug === selectedSlug.value) ?? workspace.projects[0] ?? null,
);

const projectOptions = computed<DropdownOption[]>(() =>
  workspace.projects.map((p) => ({ value: p.slug, label: p.name })),
);

const ws = computed(() => workspace.activeWorkspaceSlug ?? '');

function selectProject(slug: string): void {
  selectedSlug.value = slug;
  try {
    localStorage.setItem(PROJECT_STORAGE_KEY, slug);
  } catch {
    // ignore storage errors
  }
  void loadTree();
}

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

async function moveNodes(nodes: TreeNodeRef[], target: string | null): Promise<void> {
  const project = activeProject.value;
  if (project === null || ws.value === '') return;

  let failed = false;
  for (const node of nodes) {
    const ok =
      node.type === 'doc'
        ? await documents.move(ws.value, project.slug, node.id, target)
        : await folders.move(ws.value, project.slug, node.id, target);
    if (!ok) failed = true;
  }

  selection.clear();
  if (failed) {
    ui.showBanner(documents.error ?? folders.error ?? 'Move failed', 'error');
  }
}

async function copyNodes(nodes: TreeNodeRef[], target: string | null): Promise<void> {
  const project = activeProject.value;
  if (project === null || ws.value === '') return;

  let failed = false;
  for (const node of nodes) {
    const ok =
      node.type === 'doc'
        ? await documents.copy(ws.value, project.slug, node.id, target)
        : await folders.copy(ws.value, project.slug, node.id, target);
    if (!ok) failed = true;
  }

  selection.clear();
  if (failed) {
    ui.showBanner(documents.error ?? folders.error ?? 'Copy failed', 'error');
  }
}

// "Move to…" / "Copy to…" open a folder picker; the pending op + nodes are held
// until the user confirms a destination.
const pendingOp = ref<'move' | 'copy' | null>(null);
const pendingNodes = ref<TreeNodeRef[]>([]);

const pickerTitle = computed(() => (pendingOp.value === 'copy' ? 'Copy to…' : 'Move to…'));
const pickerConfirm = computed(() => (pendingOp.value === 'copy' ? 'Copy here' : 'Move here'));

function requestMove(nodes: TreeNodeRef[]): void {
  pendingNodes.value = nodes;
  pendingOp.value = 'move';
}

function requestCopy(nodes: TreeNodeRef[]): void {
  pendingNodes.value = nodes;
  pendingOp.value = 'copy';
}

async function onPickFolder(target: string | null): Promise<void> {
  const op = pendingOp.value;
  const nodes = pendingNodes.value;
  pendingOp.value = null;
  pendingNodes.value = [];
  if (op === 'move') await moveNodes(nodes, target);
  else if (op === 'copy') await copyNodes(nodes, target);
}

onMounted(loadTree);
watch(() => workspace.activeWorkspaceSlug, loadTree);

function openNewPage(): void {
  treeRef.value?.openNewPage();
}

defineExpose({ openNewPage });
</script>

<template>
  <div v-if="workspace.projects.length > 0">
    <div
      v-if="projectOptions.length > 1"
      style="padding: 6px 8px 8px; border-bottom: 1px solid var(--c-border); margin-bottom: 6px;"
    >
      <Dropdown
        :options="projectOptions"
        :model-value="activeProject?.slug ?? ''"
        @change="selectProject"
      />
    </div>

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
      @move-nodes="moveNodes"
      @request-move="requestMove"
      @request-copy="requestCopy"
    />
    <FolderPickerDialog
      :open="pendingOp !== null"
      :title="pickerTitle"
      :confirm-label="pickerConfirm"
      :folders="folders.folders"
      @confirm="onPickFolder"
      @cancel="pendingOp = null"
    />
  </div>
  <p
    v-else
    style="padding: 8px; font-size: var(--fs-sm); color: var(--c-muted);"
  >
    No project selected.
  </p>
</template>
