<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import { wrappedClient } from '@/api/wrapper';
import {
  getResourceCachePrincipal,
  hydrateAndRevalidateResource,
  resourceCache,
  resourceCacheEpoch,
  resourceCacheIsPurging,
} from '@/cache/cacheRuntime';
import { buildCacheKey, CACHE_CADENCE } from '@/cache/resourceCache';
import FolderPickerDialog from '@/components/notas/FolderPickerDialog.vue';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import NotesTree from '@/components/notas/NotesTree.vue';
import ErrorState from '@/components/states/ErrorState.vue';
import FreshnessStatus from '@/components/states/FreshnessStatus.vue';
import LoadingState from '@/components/states/LoadingState.vue';
import Dropdown, { type DropdownOption } from '@/components/ui/Dropdown.vue';
import { type LiveUpdateEvent, useLiveUpdates } from '@/composables/useLiveUpdates';
import { EVENT_TYPE } from '@/lib/eventTypes';
import { type NoteCatalog, noteCatalogSchema } from '@/lib/noteCatalog';
import { docKey, type TreeNodeRef } from '@/lib/notesTree';
import { collectPaged } from '@/lib/pagination';
import { useDocumentsStore } from '@/stores/documents';
import { useFoldersStore } from '@/stores/folders';
import { useNotesTabsStore } from '@/stores/notesTabs';
import { useResourceStatusStore } from '@/stores/resourceStatus';
import { useTreeSelection } from '@/stores/treeSelection';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

const PROJECT_STORAGE_KEY = 'atlas:notes-project';
const CATALOG_RETENTION_MS = 24 * 60 * 60 * 1000;

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
const tabs = useNotesTabsStore();
const ui = useUiStore();
const resourceStatus = useResourceStatusStore();

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
  workspace.projects.map((p) => ({ value: p.slug, label: p.name, icon: 'folder' })),
);

const ws = computed(() => workspace.activeWorkspaceSlug ?? '');
const catalogTarget = ref<string | null>(null);
const loading = ref(false);
const catalogError = ref<string | null>(null);
const catalogCacheUnavailable = ref(false);
let catalogSequence = 0;
let catalogPurgePending = false;
let loadedWorkspaceSlug: string | undefined;
const hasCatalog = computed(() => catalogTarget.value !== null);
const treeLoading = computed(() => loading.value && !hasCatalog.value);
const treeFolders = computed(() =>
  activeProject.value === null ? [] : folders.foldersFor(activeProject.value.slug),
);
const treeSummaries = computed(() =>
  activeProject.value === null ? [] : documents.summariesFor(activeProject.value.slug),
);
const catalogStatusKey = computed(() =>
  ws.value === '' || activeProject.value === null ? '' : `note-tree:${ws.value}:${activeProject.value.slug}`,
);
const catalogFreshnessStatus = computed(() =>
  catalogStatusKey.value === '' ? 'empty' : resourceStatus.statusFor(catalogStatusKey.value),
);

function clearCatalog(projectSlug: string | null = activeProject.value?.slug ?? null): void {
  catalogSequence += 1;
  catalogTarget.value = null;
  catalogError.value = null;

  if (projectSlug !== null) {
    folders.publishForProject(projectSlug, []);
    documents.publishSummariesForProject(projectSlug, []);
  }
}

function clearCatalogScope(): void {
  folders.clearProjectBuckets();
  documents.clearProjectBuckets();
  clearCatalog(null);
}

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
  const sequence = ++catalogSequence;
  const wsSlug = workspace.activeWorkspaceSlug;
  if (wsSlug === null) {
    // Keep `loadedWorkspaceSlug` intact: a switch nulls the active slug only
    // transiently, and resetting it here would disarm the workspace-change guard
    // below on the next pass, so the real switch would skip its `nextTick` fence.
    catalogTarget.value = null;
    await workspace.loadProjects('');
    return;
  }

  const workspaceChanged = loadedWorkspaceSlug !== undefined && loadedWorkspaceSlug !== wsSlug;
  loadedWorkspaceSlug = wsSlug;
  if (workspaceChanged) {
    await nextTick();
    if (sequence !== catalogSequence || workspace.activeWorkspaceSlug !== wsSlug) return;
  }

  if (workspace.projects.length === 0) {
    await workspace.loadProjects(wsSlug);
  }

  const project = activeProject.value;
  if (project === null) return;

  const target = `${wsSlug}:${project.slug}`;
  if (catalogTarget.value !== target) {
    catalogTarget.value = null;
  }

  const workspaceId = workspace.workspaceIdForSlug(wsSlug);
  const isCurrent = () =>
    sequence === catalogSequence &&
    workspace.activeWorkspaceSlug === wsSlug &&
    activeProject.value?.slug === project.slug;
  const key = catalogCacheUnavailable.value
    ? null
    : buildCacheKey({
        principal: getResourceCachePrincipal(),
        workspaceId,
        resourceKind: 'note-tree',
        resourceId: project.slug,
      });

  if (key === null || !resourceCache.isAvailable()) {
    loading.value = true;
    catalogError.value = null;
    const statusKey = `note-tree:${wsSlug}:${project.slug}`;
    const onlineHint = typeof navigator === 'undefined' || navigator.onLine;
    resourceStatus.beginRequest(statusKey, onlineHint);

    await Promise.all([folders.load(wsSlug, project.slug), documents.loadSummaries(wsSlug, project.slug)]);
    if (isCurrent()) {
      if (folders.error !== null || documents.error !== null) {
        catalogError.value = folders.error ?? documents.error ?? 'Failed to load notes';
        resourceStatus.recordRequestFailure(statusKey, onlineHint);
      } else {
        catalogTarget.value = target;
        resourceStatus.recordRequestSuccess(statusKey, true);
      }
    }
    if (sequence === catalogSequence) loading.value = false;
    return;
  }

  const publish = (payload: NoteCatalog): void => {
    if (!isCurrent()) return;

    folders.publishForProject(project.slug, payload.folders);
    documents.publishSummariesForProject(project.slug, payload.summaries);
    catalogTarget.value = target;
    resourceStatus.recordRequestSuccess(`note-tree:${wsSlug}:${project.slug}`, true);
  };
  let catalogDenied = false;
  const load = async (): Promise<NoteCatalog> => {
    const [folderPage, summaryPage] = await Promise.all([
      collectPaged((cursor) =>
        wrappedClient.GET('/api/workspaces/{ws}/projects/{project_slug}/folders', {
          params: {
            path: { ws: wsSlug, project_slug: project.slug },
            query: { limit: 200, ...(cursor === undefined ? {} : { cursor }) },
          },
        }),
      ),
      collectPaged((cursor) =>
        wrappedClient.GET('/api/workspaces/{ws}/projects/{project_slug}/documents', {
          params: {
            path: { ws: wsSlug, project_slug: project.slug },
            query: { limit: 200, ...(cursor === undefined ? {} : { cursor }) },
          },
        }),
      ),
    ]);

    if (folderPage.error !== undefined || summaryPage.error !== undefined) {
      const error = new Error('Failed to load notes') as Error & { status?: number };
      const source = folderPage.error ?? summaryPage.error;
      error.status = (source as { status?: number } | undefined)?.status;
      catalogDenied = error.status === 403 || error.status === 404;
      throw error;
    }

    // Board rows join the catalog once the unified-sidebar rewrite loads them
    // per project; until then the catalog schema carries an empty array so
    // cached entries validate the same way before and after that rewrite lands.
    return { folders: folderPage.items, summaries: summaryPage.items, boards: [] };
  };
  const request = {
    key,
    payloadSchema: noteCatalogSchema,
    tags: [`project:${project.slug}`],
    freshForMs: CACHE_CADENCE.catalog.freshForMs,
    activeForMs: CACHE_CADENCE.catalog.activeForMs,
    retentionForMs: CATALOG_RETENTION_MS,
    load,
    publish,
    isCurrent,
  };

  loading.value = true;
  catalogError.value = null;
  const statusKey = `note-tree:${wsSlug}:${project.slug}`;
  const onlineHint = typeof navigator === 'undefined' || navigator.onLine;
  resourceStatus.beginRequest(statusKey, onlineHint);
  const operation = hydrateAndRevalidateResource(request);
  void operation.hydration.then((hydrated) => {
    if (hydrated !== null && isCurrent()) resourceStatus.setRefreshing(statusKey);
  });
  try {
    await operation.completion;
  } catch (error) {
    if (isCurrent()) {
      const status = (error as { status?: number } | undefined)?.status;
      if (catalogDenied || status === 403 || status === 404) {
        clearCatalog(project.slug);
        loading.value = false;
      }
      catalogError.value = 'Failed to load notes';
      resourceStatus.recordRequestFailure(statusKey, onlineHint);
    }
  } finally {
    if (sequence === catalogSequence) loading.value = false;
  }
}

function openDoc(slug: string): void {
  void router.push({ name: 'notes', params: { slug } });
}

async function invalidateCatalog(projectSlug: string): Promise<void> {
  const workspaceId = workspace.workspaceIdForSlug(ws.value);
  const principal = getResourceCachePrincipal();
  if (workspaceId === null || principal === undefined) return;

  if (!(await resourceCache.purgeTags([`project:${projectSlug}`], principal, workspaceId))) {
    catalogCacheUnavailable.value = true;
    return;
  }

  if (activeProject.value?.slug === projectSlug) await loadTree();
}

async function createDoc(title: string, folderId?: string): Promise<void> {
  const project = activeProject.value;
  if (project === null || ws.value === '') return;

  const slug = await documents.create(ws.value, project.slug, title, folderId);
  if (slug !== null) {
    await invalidateCatalog(project.slug);
    openDoc(slug);
  } else if (documents.error) {
    ui.showBanner(documents.error, 'error');
  }
}

async function renameDoc(slug: string, title: string): Promise<void> {
  const project = activeProject.value;
  if (project === null || ws.value === '') return;

  const ok = await documents.rename(ws.value, project.slug, slug, title);
  if (ok) {
    await invalidateCatalog(project.slug);
  } else if (documents.error) {
    ui.showBanner(documents.error, 'error');
  }
}

async function removeDoc(slug: string): Promise<void> {
  const project = activeProject.value;
  if (project === null || ws.value === '') return;

  const workspaceId = workspace.workspaceIdForSlug(ws.value);
  const ok = await documents.remove(
    ws.value,
    project.slug,
    slug,
    workspaceId === null ? undefined : { workspaceId },
  );
  if (!ok) {
    if (documents.error) ui.showBanner(documents.error, 'error');
    return;
  }

  await invalidateCatalog(project.slug);

  // Drop the deleted note's open tab; if it was the one being viewed, move to a
  // neighbour (or the empty notes root) so we never land on a dead tab.
  const wasActive = activeSlug.value === slug;
  const next = tabs.close(ws.value, slug);
  if (wasActive) {
    void router.push(next !== null ? { name: 'notes', params: { slug: next } } : { name: 'notes' });
  }
}

async function createFolder(name: string, parentFolderId?: string): Promise<void> {
  const project = activeProject.value;
  if (project === null || ws.value === '') return;

  const ok = await folders.create(ws.value, project.slug, name, parentFolderId);
  if (ok) {
    await invalidateCatalog(project.slug);
  } else if (folders.error) {
    ui.showBanner(folders.error, 'error');
  }
}

async function renameFolder(folderId: string, name: string): Promise<void> {
  const project = activeProject.value;
  if (project === null || ws.value === '') return;

  const ok = await folders.rename(ws.value, project.slug, folderId, name);
  if (ok) {
    await invalidateCatalog(project.slug);
  } else if (folders.error) {
    ui.showBanner(folders.error, 'error');
  }
}

async function removeFolder(folderId: string): Promise<void> {
  const project = activeProject.value;
  if (project === null || ws.value === '') return;

  if (await folders.remove(ws.value, project.slug, folderId)) {
    await invalidateCatalog(project.slug);
  }
}

async function moveNodes(nodes: TreeNodeRef[], target: string | null): Promise<void> {
  const project = activeProject.value;
  if (project === null || ws.value === '') return;

  let failed = false;
  let mutated = false;
  for (const node of nodes) {
    const ok =
      node.type === 'doc'
        ? await documents.move(ws.value, project.slug, node.id, target)
        : await folders.move(ws.value, project.slug, node.id, target);
    if (!ok) failed = true;
    else mutated = true;
  }

  if (mutated) await invalidateCatalog(project.slug);

  selection.clear();
  if (failed) {
    ui.showBanner(documents.error ?? folders.error ?? 'Move failed', 'error');
  }
}

async function copyNodes(nodes: TreeNodeRef[], target: string | null): Promise<void> {
  const project = activeProject.value;
  if (project === null || ws.value === '') return;

  let failed = false;
  let mutated = false;
  for (const node of nodes) {
    const ok =
      node.type === 'doc'
        ? await documents.copy(ws.value, project.slug, node.id, target)
        : await folders.copy(ws.value, project.slug, node.id, target);
    if (!ok) failed = true;
    else mutated = true;
  }

  if (mutated) await invalidateCatalog(project.slug);

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

// Refreshes the active project's note list silently (no loader flash) when
// another actor creates, moves, deletes, or updates a document. A
// `document.updated` only refreshes summary metadata (title, updated_at); the
// open editor is CAS-managed and never live-replaced here.
function reloadNotesLive(): void {
  const project = activeProject.value;
  if (project === null || ws.value === '') return;
  void loadTree();
}

function onLiveEvent(evt: LiveUpdateEvent): void {
  switch (evt.type) {
    case EVENT_TYPE.DOCUMENT_CREATED:
    case EVENT_TYPE.DOCUMENT_UPDATED:
    case EVENT_TYPE.DOCUMENT_MOVED:
    case EVENT_TYPE.DOCUMENT_DELETED:
    // The server does not yet emit board.updated/board.moved events (rename and
    // move — including the new folder-move endpoint — publish nothing), so only
    // the two board events that actually exist are wired here.
    case EVENT_TYPE.BOARD_CREATED:
    case EVENT_TYPE.BOARD_DELETED:
      reloadNotesLive();
      break;

    default:
      break;
  }
}

useLiveUpdates(ws, { onEvent: onLiveEvent, onResync: () => void loadTree() });

onMounted(loadTree);
watch(
  [() => workspace.activeWorkspaceSlug, resourceCacheEpoch, resourceCacheIsPurging],
  () => {
    clearCatalogScope();
    if (resourceCacheIsPurging.value) {
      catalogPurgePending = true;
      return;
    }
    if (catalogPurgePending && getResourceCachePrincipal() === undefined) return;

    catalogPurgePending = false;
    void loadTree();
  },
  { flush: 'sync' },
);

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

      <LoadingState v-if="treeLoading" label="Loading notes…" />
      <ErrorState
        v-else-if="catalogError !== null && !hasCatalog"
        title="Couldn’t load notes"
        :hint="catalogError"
        @retry="loadTree"
      />
      <NotesTree
      v-else-if="activeProject && hasCatalog"
      ref="treeRef"
      :project-name="activeProject.name"
       :folders="treeFolders"
       :docs="treeSummaries"
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
     <FreshnessStatus
        v-if="hasCatalog || catalogError !== null"
       :status="catalogFreshnessStatus"
       @retry="loadTree"
     />
    <FolderPickerDialog
      :open="pendingOp !== null"
      :title="pickerTitle"
      :confirm-label="pickerConfirm"
       :folders="treeFolders"
      @confirm="onPickFolder"
      @cancel="pendingOp = null"
    />
  </div>
  <ErrorState
    v-else-if="workspace.projectsError !== null"
    title="Couldn’t load projects"
    :hint="workspace.projectsError"
    @retry="loadTree"
  />
  <p
    v-else
    style="padding: 8px; font-size: var(--fs-sm); color: var(--c-muted);"
  >
    No project selected.
  </p>
</template>
