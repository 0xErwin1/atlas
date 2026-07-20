<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue';
import { useRouter } from 'vue-router';
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
import ContextMenu, { type MenuItem } from '@/components/ui/ContextMenu.vue';
import Row from '@/components/ui/Row.vue';
import { useContextMenu } from '@/composables/useContextMenu';
import { type LiveUpdateEvent, useLiveUpdates } from '@/composables/useLiveUpdates';
import { EVENT_TYPE } from '@/lib/eventTypes';
import { type NoteCatalog, noteCatalogSchema } from '@/lib/noteCatalog';
import { docKey, type TreeNodeRef } from '@/lib/notesTree';
import { collectPaged } from '@/lib/pagination';
import { useBoardsStore } from '@/stores/boards';
import { useDocumentsStore } from '@/stores/documents';
import { useFoldersStore } from '@/stores/folders';
import { useNotesTabsStore } from '@/stores/notesTabs';
import { useResourceStatusStore } from '@/stores/resourceStatus';
import { useTreeSelection } from '@/stores/treeSelection';
import { useUiStore } from '@/stores/ui';
import type { ProjectSummary } from '@/stores/workspace';
import { useWorkspaceStore } from '@/stores/workspace';

const CATALOG_RETENTION_MS = 24 * 60 * 60 * 1000;

const props = defineProps<{
  project: ProjectSummary;
  activeSlug: string | null;
  activeBoardId: string | null;
}>();

const router = useRouter();
const workspace = useWorkspaceStore();
const treeRef = ref<InstanceType<typeof NotesTree> | null>(null);
const folders = useFoldersStore();
const documents = useDocumentsStore();
const boards = useBoardsStore();
const selection = useTreeSelection();
const tabs = useNotesTabsStore();
const ui = useUiStore();
const resourceStatus = useResourceStatusStore();

const expanded = ref(true);

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
const treeFolders = computed(() => folders.foldersFor(props.project.slug));
const treeSummaries = computed(() => documents.summariesFor(props.project.slug));
const treeBoards = computed(() => boards.boardsFor(props.project.slug));
const catalogStatusKey = computed(() =>
  ws.value === '' ? '' : `note-tree:${ws.value}:${props.project.slug}`,
);
const catalogFreshnessStatus = computed(() =>
  catalogStatusKey.value === '' ? 'empty' : resourceStatus.statusFor(catalogStatusKey.value),
);

function clearCatalog(): void {
  catalogSequence += 1;
  catalogTarget.value = null;
  catalogError.value = null;

  folders.publishForProject(props.project.slug, []);
  documents.publishSummariesForProject(props.project.slug, []);
  boards.publishForProject(props.project.slug, []);
}

async function loadTree(): Promise<void> {
  const sequence = ++catalogSequence;
  const wsSlug = workspace.activeWorkspaceSlug;
  if (wsSlug === null) {
    catalogTarget.value = null;
    return;
  }

  const workspaceChanged = loadedWorkspaceSlug !== undefined && loadedWorkspaceSlug !== wsSlug;
  loadedWorkspaceSlug = wsSlug;
  if (workspaceChanged) {
    await nextTick();
    if (sequence !== catalogSequence || workspace.activeWorkspaceSlug !== wsSlug) return;
  }

  const project = props.project;
  const target = `${wsSlug}:${project.slug}`;
  if (catalogTarget.value !== target) {
    catalogTarget.value = null;
  }

  const workspaceId = workspace.workspaceIdForSlug(wsSlug);
  const isCurrent = () => sequence === catalogSequence && workspace.activeWorkspaceSlug === wsSlug;
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

    await Promise.all([
      folders.load(wsSlug, project.slug),
      documents.loadSummaries(wsSlug, project.slug),
      boards.loadBoardsForProject(wsSlug, project.slug),
    ]);
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
    boards.publishForProject(project.slug, payload.boards);
    catalogTarget.value = target;
    resourceStatus.recordRequestSuccess(`note-tree:${wsSlug}:${project.slug}`, true);
  };
  let catalogDenied = false;
  const load = async (): Promise<NoteCatalog> => {
    const [folderPage, summaryPage, boardPage] = await Promise.all([
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
      collectPaged((cursor) =>
        wrappedClient.GET('/api/workspaces/{ws}/projects/{project_slug}/boards', {
          params: {
            path: { ws: wsSlug, project_slug: project.slug },
            query: { limit: 200, ...(cursor === undefined ? {} : { cursor }) },
          },
        }),
      ),
    ]);

    if (folderPage.error !== undefined || summaryPage.error !== undefined || boardPage.error !== undefined) {
      const error = new Error('Failed to load notes') as Error & { status?: number };
      const source = folderPage.error ?? summaryPage.error ?? boardPage.error;
      error.status = (source as { status?: number } | undefined)?.status;
      catalogDenied = error.status === 403 || error.status === 404;
      throw error;
    }

    return { folders: folderPage.items, summaries: summaryPage.items, boards: boardPage.items };
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
        clearCatalog();
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

function openBoard(boardId: string): void {
  void router.push({ name: 'tasks', params: { boardId } });
}

async function invalidateCatalog(): Promise<void> {
  const workspaceId = workspace.workspaceIdForSlug(ws.value);
  const principal = getResourceCachePrincipal();
  if (workspaceId === null || principal === undefined) return;

  if (!(await resourceCache.purgeTags([`project:${props.project.slug}`], principal, workspaceId))) {
    catalogCacheUnavailable.value = true;
    return;
  }

  await loadTree();
}

async function createDoc(title: string, folderId?: string): Promise<void> {
  if (ws.value === '') return;

  const slug = await documents.create(ws.value, props.project.slug, title, folderId);
  if (slug !== null) {
    await invalidateCatalog();
    openDoc(slug);
  } else if (documents.error) {
    ui.showBanner(documents.error, 'error');
  }
}

async function renameDoc(slug: string, title: string): Promise<void> {
  if (ws.value === '') return;

  const ok = await documents.rename(ws.value, props.project.slug, slug, title);
  if (ok) {
    await invalidateCatalog();
  } else if (documents.error) {
    ui.showBanner(documents.error, 'error');
  }
}

async function removeDoc(slug: string): Promise<void> {
  if (ws.value === '') return;

  const workspaceId = workspace.workspaceIdForSlug(ws.value);
  const ok = await documents.remove(
    ws.value,
    props.project.slug,
    slug,
    workspaceId === null ? undefined : { workspaceId },
  );
  if (!ok) {
    if (documents.error) ui.showBanner(documents.error, 'error');
    return;
  }

  await invalidateCatalog();

  const wasActive = props.activeSlug === slug;
  const next = tabs.close(ws.value, slug);
  if (wasActive) {
    void router.push(next !== null ? { name: 'notes', params: { slug: next } } : { name: 'notes' });
  }
}

async function createFolder(name: string, parentFolderId?: string): Promise<void> {
  if (ws.value === '') return;

  const ok = await folders.create(ws.value, props.project.slug, name, parentFolderId);
  if (ok) {
    await invalidateCatalog();
  } else if (folders.error) {
    ui.showBanner(folders.error, 'error');
  }
}

async function renameFolder(folderId: string, name: string): Promise<void> {
  if (ws.value === '') return;

  const ok = await folders.rename(ws.value, props.project.slug, folderId, name);
  if (ok) {
    await invalidateCatalog();
  } else if (folders.error) {
    ui.showBanner(folders.error, 'error');
  }
}

async function removeFolder(folderId: string): Promise<void> {
  if (ws.value === '') return;

  if (await folders.remove(ws.value, props.project.slug, folderId)) {
    await invalidateCatalog();
  }
}

async function createBoard(name: string, folderId?: string): Promise<void> {
  if (ws.value === '') return;

  const id = await boards.createBoard(ws.value, props.project.slug, name, folderId ?? null);
  if (id !== null) {
    await invalidateCatalog();
    openBoard(id);
  } else if (boards.error) {
    ui.showBanner(boards.error, 'error');
  }
}

async function renameBoard(boardId: string, name: string): Promise<void> {
  if (ws.value === '') return;

  const ok = await boards.renameBoard(ws.value, props.project.slug, boardId, name);
  if (ok) {
    await invalidateCatalog();
  } else if (boards.error) {
    ui.showBanner(boards.error, 'error');
  }
}

async function removeBoard(boardId: string): Promise<void> {
  if (ws.value === '') return;

  const wasActive = props.activeBoardId === boardId;
  const ok = await boards.removeBoard(ws.value, props.project.slug, boardId);
  if (!ok) {
    if (boards.error) ui.showBanner(boards.error, 'error');
    return;
  }

  await invalidateCatalog();
  if (wasActive) void router.push({ name: 'tasks' });
}

async function moveNodes(nodes: TreeNodeRef[], target: string | null): Promise<void> {
  if (ws.value === '') return;

  let failed = false;
  let mutated = false;
  for (const node of nodes) {
    const ok =
      node.type === 'doc'
        ? await documents.move(ws.value, props.project.slug, node.id, target)
        : node.type === 'board'
          ? await boards.moveBoard(ws.value, props.project.slug, node.id, target)
          : await folders.move(ws.value, props.project.slug, node.id, target);
    if (!ok) failed = true;
    else mutated = true;
  }

  if (mutated) await invalidateCatalog();

  selection.clear();
  if (failed) {
    ui.showBanner(documents.error ?? folders.error ?? boards.error ?? 'Move failed', 'error');
  }
}

async function copyNodes(nodes: TreeNodeRef[], target: string | null): Promise<void> {
  if (ws.value === '') return;

  let failed = false;
  let mutated = false;
  for (const node of nodes) {
    // Boards have no copy operation; only documents and folders are copyable.
    if (node.type === 'board') continue;
    const ok =
      node.type === 'doc'
        ? await documents.copy(ws.value, props.project.slug, node.id, target)
        : await folders.copy(ws.value, props.project.slug, node.id, target);
    if (!ok) failed = true;
    else mutated = true;
  }

  if (mutated) await invalidateCatalog();

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

// Refreshes this project's catalog silently when another actor mutates a
// document or board somewhere in the workspace. Every space revalidates its own
// cache entry; the freshness window dedups redundant refetches.
function reloadNotesLive(): void {
  if (ws.value === '') return;
  void loadTree();
}

function onLiveEvent(evt: LiveUpdateEvent): void {
  switch (evt.type) {
    case EVENT_TYPE.DOCUMENT_CREATED:
    case EVENT_TYPE.DOCUMENT_UPDATED:
    case EVENT_TYPE.DOCUMENT_MOVED:
    case EVENT_TYPE.DOCUMENT_DELETED:
    case EVENT_TYPE.BOARD_CREATED:
    case EVENT_TYPE.BOARD_UPDATED:
    case EVENT_TYPE.BOARD_DELETED:
    case EVENT_TYPE.BOARD_MOVED:
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
    clearCatalog();
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

const { open: menuOpen, x: menuX, y: menuY, openAt, close: closeMenu } = useContextMenu();

// Creation from the space header: expand the space, then open the tree's inline
// input so the new item is typed in this project's context.
async function startCreate(kind: 'page' | 'board' | 'folder'): Promise<void> {
  expanded.value = true;
  await nextTick();
  if (kind === 'page') treeRef.value?.openNewPage();
  else if (kind === 'board') treeRef.value?.openNewBoard();
  else treeRef.value?.openNewFolder();
}

const headerMenuItems = computed<MenuItem[]>(() => [
  { header: true, label: props.project.name },
  { label: 'New page', icon: 'file-plus', action: () => void startCreate('page') },
  { label: 'New board', icon: 'columns-3', action: () => void startCreate('board') },
  { label: 'New folder', icon: 'folder-plus', action: () => void startCreate('folder') },
]);

function openHeaderMenu(event: MouseEvent): void {
  openAt(event);
}

function toggleExpanded(): void {
  expanded.value = !expanded.value;
}

defineExpose({
  reload: loadTree,
  startNewPage: () => void startCreate('page'),
  startNewBoard: () => void startCreate('board'),
});
</script>

<template>
  <div>
    <Row
      :label="project.name"
      :icon="expanded ? 'folder-open' : 'folder'"
      chevron
      :open="expanded"
      menu
      @click="toggleExpanded"
      @menu="openHeaderMenu"
      @contextmenu.prevent.stop="openHeaderMenu"
    />

    <template v-if="expanded">
      <LoadingState v-if="treeLoading" label="Loading notes…" />
      <ErrorState
        v-else-if="catalogError !== null && !hasCatalog"
        title="Couldn’t load notes"
        :hint="catalogError"
        @retry="loadTree"
      />
      <NotesTree
        v-else-if="hasCatalog"
        ref="treeRef"
        :project-name="project.name"
        :folders="treeFolders"
        :docs="treeSummaries"
        :boards="treeBoards"
        :active-slug="activeSlug"
        :active-board-id="activeBoardId"
        @select-doc="openDoc"
        @select-board="openBoard"
        @create-doc="createDoc"
        @rename-doc="renameDoc"
        @remove-doc="removeDoc"
        @create-folder="createFolder"
        @rename-folder="renameFolder"
        @remove-folder="removeFolder"
        @create-board="createBoard"
        @rename-board="renameBoard"
        @remove-board="removeBoard"
        @move-nodes="moveNodes"
        @request-move="requestMove"
        @request-copy="requestCopy"
      />
      <FreshnessStatus
        v-if="hasCatalog || catalogError !== null"
        :status="catalogFreshnessStatus"
        @retry="loadTree"
      />
    </template>

    <FolderPickerDialog
      :open="pendingOp !== null"
      :title="pickerTitle"
      :confirm-label="pickerConfirm"
      :folders="treeFolders"
      @confirm="onPickFolder"
      @cancel="pendingOp = null"
    />

    <ContextMenu
      :open="menuOpen"
      :x="menuX"
      :y="menuY"
      :items="headerMenuItems"
      @close="closeMenu"
    />
  </div>
</template>
