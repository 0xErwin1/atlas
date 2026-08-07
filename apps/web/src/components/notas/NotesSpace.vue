<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { useRouter } from 'vue-router';
import { wrappedClient } from '@/api/wrapper';
import {
  getResourceCachePrincipal,
  hydrateAndRevalidateResource,
  resourceCache,
  resourceCacheEpoch,
  resourceCacheIsPurging,
} from '@/cache/cacheRuntime';
import { projectCatalogTag } from '@/cache/cacheInvalidation';
import { buildCacheKey, CACHE_CADENCE } from '@/cache/resourceCache';
import FolderPickerDialog from '@/components/notas/FolderPickerDialog.vue';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import NotesTree from '@/components/notas/NotesTree.vue';
import ErrorState from '@/components/states/ErrorState.vue';
import FreshnessStatus from '@/components/states/FreshnessStatus.vue';
import LoadingState from '@/components/states/LoadingState.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import ContextMenu, { type MenuItem } from '@/components/ui/ContextMenu.vue';
import Row from '@/components/ui/Row.vue';
import { useContextMenu } from '@/composables/useContextMenu';
import { type LiveUpdateEvent, useLiveUpdates } from '@/composables/useLiveUpdates';
import { useProjectDeletion } from '@/composables/useProjectDeletion';
import { routeAfterClose } from '@/lib/docsTabs';
import { EVENT_TYPE, eventString, type LiveEnvelope } from '@/lib/eventTypes';
import { type NoteCatalog, noteCatalogSchema } from '@/lib/noteCatalog';
import { docKey, type TreeNodeRef } from '@/lib/notesTree';
import { collectPaged } from '@/lib/pagination';
import { useBoardsStore } from '@/stores/boards';
import { useDocumentsStore } from '@/stores/documents';
import { useFoldersStore } from '@/stores/folders';
import { useLastViewedStore } from '@/stores/lastViewed';
import { useNotesTabsStore } from '@/stores/notesTabs';
import { useResourceStatusStore } from '@/stores/resourceStatus';
import { useTreeSelection } from '@/stores/treeSelection';
import { useUiStore } from '@/stores/ui';
import { useUiStateStore } from '@/stores/uiState';
import type { ProjectSummary } from '@/stores/workspace';
import { useWorkspaceStore } from '@/stores/workspace';

const CATALOG_RETENTION_MS = 24 * 60 * 60 * 1000;

// Live events arrive in bursts (an SSE dispatch invalidates the cache for every
// event, and a single user action can emit several). Each event is coalesced into
// a single trailing catalog reload so a burst of M events triggers one reload,
// not M. The max-wait bounds the delay when events keep arriving back-to-back.
const LIVE_RELOAD_DEBOUNCE_MS = 250;
const LIVE_RELOAD_MAX_WAIT_MS = 1000;

const props = defineProps<{
  project: ProjectSummary;
  activeSlug: string | null;
  activeBoardId: string | null;
}>();

// Emitted once, when this space's first catalog load settles (ready or error),
// so the sidebar can gate its tree behind every space's initial readiness and
// show a single loader instead of a phased pop-in. Later background
// revalidations never re-emit it.
const emit = defineEmits<{ 'initial-settled': [] }>();

const router = useRouter();
const workspace = useWorkspaceStore();
const treeRef = ref<InstanceType<typeof NotesTree> | null>(null);
const folders = useFoldersStore();
const documents = useDocumentsStore();
const boards = useBoardsStore();
const selection = useTreeSelection();
const tabs = useNotesTabsStore();
const { deleteProject: runProjectDeletion } = useProjectDeletion();
const ui = useUiStore();
const resourceStatus = useResourceStatusStore();
const uiState = useUiStateStore();

const expanded = computed(() =>
  props.project.id === undefined
    ? true
    : !uiState.isProjectCollapsed(props.project.workspace_id, props.project.id),
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

interface CatalogLoadContext {
  sequence: number;
  wsSlug: string;
  project: ProjectSummary;
  target: string;
  statusKey: string;
  /**
   * A background catch-up rather than a user-visible load: it must not raise the
   * loading flag, flap the freshness row, or replace a rendered tree with an
   * error. The published catalog still swaps in atomically when it lands.
   */
  silent: boolean;
  isCurrent: () => boolean;
}

interface LoadTreeOptions {
  silent?: boolean;
}

async function loadTree(options: LoadTreeOptions = {}): Promise<void> {
  const sequence = ++catalogSequence;
  const wsSlug = workspace.activeWorkspaceSlug;
  if (wsSlug === null) {
    catalogTarget.value = null;
    settleInitial();
    return;
  }

  const workspaceChanged = loadedWorkspaceSlug !== undefined && loadedWorkspaceSlug !== wsSlug;
  loadedWorkspaceSlug = wsSlug;
  if (workspaceChanged) {
    await nextTick();
    if (sequence !== catalogSequence || workspace.activeWorkspaceSlug !== wsSlug) return;
  }

  const project = props.project;
  const workspaceId = workspace.workspaceIdForSlug(wsSlug);
  // A synchronous workspace change can reach this still-mounted space before
  // Vue removes its old project row. Do not issue that stale catalog request.
  if (workspaceId !== null && project.workspace_id !== workspaceId) {
    catalogTarget.value = null;
    settleInitial();
    return;
  }

  const target = `${wsSlug}:${project.slug}`;
  if (catalogTarget.value !== target) {
    catalogTarget.value = null;
  }

  // A silent catch-up on a tree that is not on screen yet has nothing to preserve,
  // so it falls back to a visible load rather than settling nothing.
  const context: CatalogLoadContext = {
    sequence,
    wsSlug,
    project,
    target,
    statusKey: `note-tree:${wsSlug}:${project.slug}`,
    silent: options.silent === true && catalogTarget.value === target,
    isCurrent: () => sequence === catalogSequence && workspace.activeWorkspaceSlug === wsSlug,
  };

  const key = catalogCacheUnavailable.value
    ? null
    : buildCacheKey({
        principal: getResourceCachePrincipal(),
        workspaceId,
        resourceKind: 'note-tree',
        resourceId: project.slug,
      });

  if (key === null || !resourceCache.isAvailable()) {
    await loadCatalogDegraded(context);
    return;
  }

  await loadCatalogCached(context, key);
}

// Degraded path: the resource cache is unavailable, so each of folders,
// documents and boards is fetched directly and the catalog only publishes when
// all three succeed. Boards are gated on their dedicated load-error signal
// (never the shared `boards.error` action channel, which an unrelated mutation
// may have set) so a boards-only failure surfaces the error state instead of
// silently publishing a catalog with stale or missing boards.
async function loadCatalogDegraded(ctx: CatalogLoadContext): Promise<void> {
  if (!ctx.silent) {
    loading.value = true;
    catalogError.value = null;
  }
  const onlineHint = typeof navigator === 'undefined' || navigator.onLine;
  if (!ctx.silent) resourceStatus.beginRequest(ctx.statusKey, onlineHint);

  const [, , boardsError] = await Promise.all([
    folders.load(ctx.wsSlug, ctx.project.slug),
    documents.loadSummaries(ctx.wsSlug, ctx.project.slug),
    boards.loadBoardsForProject(ctx.wsSlug, ctx.project.slug),
  ]);

  if (ctx.isCurrent()) {
    const failure = folders.error ?? documents.error ?? boardsError;
    if (failure !== null) {
      // A failed background catch-up keeps the rendered tree; only the freshness
      // row reports that the update did not land.
      if (!ctx.silent) catalogError.value = failure;
      resourceStatus.recordRequestFailure(ctx.statusKey, onlineHint);
    } else {
      catalogTarget.value = ctx.target;
      resourceStatus.recordRequestSuccess(ctx.statusKey, true);
    }
  }

  // Cleared even for a silent load: this sequence supersedes any in-flight
  // visible load, whose own `finally` will no longer match.
  if (ctx.sequence === catalogSequence) loading.value = false;
}

// Cached path: hydrate synchronously from the resource cache when present, then
// revalidate over the network. `publish` swaps folders, documents and boards in
// atomically so the tree never shows a half-updated catalog.
async function loadCatalogCached(ctx: CatalogLoadContext, key: string): Promise<void> {
  const { wsSlug, project } = ctx;

  const publish = (payload: NoteCatalog): void => {
    if (!ctx.isCurrent()) return;

    folders.publishForProject(project.slug, payload.folders);
    documents.publishSummariesForProject(project.slug, payload.summaries);
    boards.publishForProject(project.slug, payload.boards);
    catalogTarget.value = ctx.target;
    resourceStatus.recordRequestSuccess(ctx.statusKey, true);
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

  // Tagged by slug for local mutations (which know it) and by UUID for live
  // events (whose envelopes only ever route by UUID).
  const tags = [`project:${project.slug}`];
  if (project.id !== undefined && project.id !== '') tags.push(projectCatalogTag(project.id));

  const request = {
    key,
    payloadSchema: noteCatalogSchema,
    tags,
    freshForMs: CACHE_CADENCE.catalog.freshForMs,
    activeForMs: CACHE_CADENCE.catalog.activeForMs,
    retentionForMs: CATALOG_RETENTION_MS,
    load,
    publish,
    isCurrent: ctx.isCurrent,
  };

  if (!ctx.silent) {
    loading.value = true;
    catalogError.value = null;
  }
  const onlineHint = typeof navigator === 'undefined' || navigator.onLine;
  if (!ctx.silent) resourceStatus.beginRequest(ctx.statusKey, onlineHint);
  const operation = hydrateAndRevalidateResource(request);
  void operation.hydration.then((hydrated) => {
    if (hydrated !== null && ctx.isCurrent() && !ctx.silent) resourceStatus.setRefreshing(ctx.statusKey);
  });
  try {
    await operation.completion;
  } catch (error) {
    if (ctx.isCurrent()) {
      const status = (error as { status?: number } | undefined)?.status;
      // Access loss is authoritative even in the background: the tree must not
      // keep showing a project this principal may no longer read.
      if (catalogDenied || status === 403 || status === 404) {
        clearCatalog();
        loading.value = false;
        catalogError.value = 'Failed to load notes';
      } else if (!ctx.silent) {
        catalogError.value = 'Failed to load notes';
      }
      resourceStatus.recordRequestFailure(ctx.statusKey, onlineHint);
    }
  } finally {
    if (ctx.sequence === catalogSequence) loading.value = false;
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
  const next = tabs.close(ws.value, { kind: 'doc', id: slug });
  if (wasActive) {
    void router.push(routeAfterClose(next));
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
  const next = tabs.close(ws.value, { kind: 'board', id: boardId });
  if (wasActive) void router.push(routeAfterClose(next));
}

async function deleteProject(): Promise<void> {
  if (ws.value === '' || deletingProject.value) return;

  deletingProject.value = true;
  const ok = await runProjectDeletion(props.project);
  deletingProject.value = false;
  if (ok) deleteProjectOpen.value = false;
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
// document or board in this project. Each relevant live event is coalesced into
// one trailing reload (see the debounce constants) so a burst of events triggers
// a single catalog refetch, not one per event. Cross-project events are filtered
// out entirely: with N spaces mounted, only the space whose project the event
// targets reloads, instead of every space refetching on every workspace event.
let liveReloadTrailingTimer: ReturnType<typeof setTimeout> | null = null;
let liveReloadMaxWaitTimer: ReturnType<typeof setTimeout> | null = null;

function clearLiveReloadTimers(): void {
  if (liveReloadTrailingTimer !== null) {
    clearTimeout(liveReloadTrailingTimer);
    liveReloadTrailingTimer = null;
  }
  if (liveReloadMaxWaitTimer !== null) {
    clearTimeout(liveReloadMaxWaitTimer);
    liveReloadMaxWaitTimer = null;
  }
}

function flushLiveReload(): void {
  clearLiveReloadTimers();
  if (ws.value === '') return;
  void loadTree();
}

function scheduleLiveReload(): void {
  if (ws.value === '') return;

  if (liveReloadTrailingTimer !== null) clearTimeout(liveReloadTrailingTimer);
  liveReloadTrailingTimer = setTimeout(flushLiveReload, LIVE_RELOAD_DEBOUNCE_MS);

  if (liveReloadMaxWaitTimer === null) {
    liveReloadMaxWaitTimer = setTimeout(flushLiveReload, LIVE_RELOAD_MAX_WAIT_MS);
  }
}

// Whether a live event concerns this space's project. Filtering only kicks in
// when both the event's `project_id` and this project's id are known; a missing
// id falls back to reloading so a payload without routing metadata is never
// silently dropped.
function eventTargetsThisProject(envelope: LiveEnvelope): boolean {
  const eventProjectId = envelope.project_id;
  const ownProjectId = props.project.id;
  if (
    typeof eventProjectId !== 'string' ||
    eventProjectId === '' ||
    ownProjectId === undefined ||
    ownProjectId === ''
  ) {
    return true;
  }

  return eventProjectId === ownProjectId;
}

type DocumentCreatedEvent = {
  documentId: string;
  slug: string;
  title: string;
  projectId: string | undefined;
};

function parseDocumentCreatedEvent(data: unknown): DocumentCreatedEvent | null {
  const documentId = eventString(data, 'document_id');
  const slug = eventString(data, 'slug');
  const title = eventString(data, 'title');
  if (
    documentId === undefined ||
    documentId === '' ||
    slug === undefined ||
    slug === '' ||
    title === undefined ||
    title === ''
  ) {
    return null;
  }

  return { documentId, slug, title, projectId: eventString(data, 'project_id') };
}

function isCurrentCreatedDocumentTarget(wsSlug: string, project: ProjectSummary): boolean {
  return ws.value === wsSlug && props.project.id === project.id && props.project.slug === project.slug;
}

async function reconcileCreatedDocument(evt: LiveUpdateEvent): Promise<void> {
  const created = parseDocumentCreatedEvent(evt.data);
  if (created === null) {
    scheduleLiveReload();
    return;
  }

  const project = props.project;
  if (created.projectId !== undefined && project.id !== undefined && created.projectId !== project.id) return;

  const wsSlug = ws.value;
  if (wsSlug === '') return;

  const fetched = await documents.fetchSummary(wsSlug, created.slug);
  if (!isCurrentCreatedDocumentTarget(wsSlug, project)) return;

  if (
    fetched === null ||
    fetched.summary.id !== created.documentId ||
    fetched.summary.slug !== created.slug ||
    (fetched.projectId !== null && project.id !== undefined && fetched.projectId !== project.id)
  ) {
    scheduleLiveReload();
    return;
  }

  documents.upsertSummary(project.slug, fetched.summary);
}

/**
 * Refreshes one already-listed document's catalog row.
 *
 * `document.updated` carries only the document's id and revision, so the row's
 * title has to be re-read — but that is one document request, not the project's
 * whole three-endpoint catalog. A document this space does not list is not ours
 * to reconcile, and is dropped rather than turned into a reload.
 */
async function reconcileUpdatedDocument(documentId: string): Promise<void> {
  const project = props.project;
  const known = documents.summaryById(project.slug, documentId);
  if (known === undefined) return;

  const knownSlug = known.slug;
  const wsSlug = ws.value;
  if (wsSlug === '' || knownSlug == null || knownSlug === '') return;

  const fetched = await documents.fetchSummary(wsSlug, knownSlug);
  if (!isCurrentCreatedDocumentTarget(wsSlug, project)) return;

  if (fetched === null || fetched.summary.id !== documentId) {
    scheduleLiveReload();
    return;
  }

  documents.upsertSummary(project.slug, fetched.summary);
}

/**
 * Applies a document move. A move within this project re-parents the row in
 * place; a move out drops it. A move *into* this project is the only case that
 * needs the catalog, because the document was never listed here to begin with.
 */
function reconcileMovedDocument(evt: LiveUpdateEvent): void {
  const documentId = eventString(evt.data, 'document_id');
  if (documentId === undefined || documentId === '') {
    scheduleLiveReload();
    return;
  }

  const project = props.project;
  const eventProjectId = eventString(evt.data, 'project_id');
  const listed = documents.summaryById(project.slug, documentId) !== undefined;

  if (eventProjectId !== undefined && project.id !== undefined && eventProjectId !== project.id) {
    if (listed) documents.removeSummaryById(project.slug, documentId);
    return;
  }

  if (!listed) {
    scheduleLiveReload();
    return;
  }

  documents.moveSummaryById(project.slug, documentId, eventString(evt.data, 'to_folder_id') ?? null);
}

function reconcileMovedBoard(evt: LiveUpdateEvent): void {
  const boardId = eventString(evt.data, 'board_id');
  if (boardId === undefined || boardId === '') {
    scheduleLiveReload();
    return;
  }

  const project = props.project;
  const eventProjectId = eventString(evt.data, 'project_id');
  const listed = boards.boardsFor(project.slug).some((board) => board.id === boardId);

  if (eventProjectId !== undefined && project.id !== undefined && eventProjectId !== project.id) {
    if (listed) boards.removeFromProject(project.slug, boardId);
    return;
  }

  if (!listed) {
    scheduleLiveReload();
    return;
  }

  boards.moveInProject(project.slug, boardId, eventString(evt.data, 'to_folder_id') ?? null);
}

/**
 * Removes a deleted node from the tree without a refetch. An id this space does
 * not list belongs to another project, so nothing is reloaded for it.
 */
function reconcileDeletion(evt: LiveUpdateEvent, kind: 'board' | 'document'): void {
  const id = eventString(evt.data, kind === 'document' ? 'document_id' : 'board_id');
  if (id === undefined || id === '') {
    scheduleLiveReload();
    return;
  }

  if (kind === 'document') documents.removeSummaryById(props.project.slug, id);
  else boards.removeFromProject(props.project.slug, id);
}

function onLiveEvent(evt: LiveUpdateEvent): void {
  if (!eventTargetsThisProject(evt.envelope)) return;

  switch (evt.type) {
    case EVENT_TYPE.DOCUMENT_CREATED:
      void reconcileCreatedDocument(evt);
      break;

    case EVENT_TYPE.DOCUMENT_UPDATED: {
      const documentId = eventString(evt.data, 'document_id');
      if (documentId === undefined || documentId === '') scheduleLiveReload();
      else void reconcileUpdatedDocument(documentId);
      break;
    }

    case EVENT_TYPE.DOCUMENT_MOVED:
      reconcileMovedDocument(evt);
      break;

    case EVENT_TYPE.DOCUMENT_DELETED:
      reconcileDeletion(evt, 'document');
      break;

    case EVENT_TYPE.BOARD_DELETED:
      reconcileDeletion(evt, 'board');
      break;

    case EVENT_TYPE.BOARD_MOVED:
      reconcileMovedBoard(evt);
      break;

    // A created board carries no folder placement and a renamed board carries no
    // new name, so neither can be placed from the payload alone.
    case EVENT_TYPE.BOARD_CREATED:
    case EVENT_TYPE.BOARD_UPDATED:
      scheduleLiveReload();
      break;

    default:
      break;
  }
}

// A benign reconnect catches up in the background: the rendered tree stays put
// and the refreshed catalog swaps in atomically. Only a real desync — where the
// cached catalog may be arbitrarily stale — pays for a visible reload.
useLiveUpdates(ws, {
  onEvent: onLiveEvent,
  onResync: (reason) => void loadTree({ silent: reason === 'reconnect' }),
});

onBeforeUnmount(clearLiveReloadTimers);

// The sidebar holds its tree behind a loader until every space reports its
// initial load settled, so this must fire on *every* terminal path of that first
// load — including the ones that return before a request is ever issued. A path
// that settles nothing latches the whole sidebar on "Loading…" permanently.
const initialSettledEmitted = ref(false);

function settleInitial(): void {
  if (initialSettledEmitted.value) return;

  initialSettledEmitted.value = true;
  emit('initial-settled');
}

watch(loading, (now, prev) => {
  if (prev && !now) settleInitial();
});

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
const deleteProjectOpen = ref(false);
const deletingProject = ref(false);

// Creation from the space header: expand the space, then open the tree's inline
// input so the new item is typed in this project's context.
async function startCreate(kind: 'page' | 'board' | 'folder'): Promise<void> {
  if (props.project.id !== undefined) {
    uiState.setProjectCollapsed(props.project.workspace_id, props.project.id, false);
  }
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
  { sep: true },
  { label: 'Delete project', icon: 'trash-2', danger: true, action: () => (deleteProjectOpen.value = true) },
]);

function openHeaderMenu(event: MouseEvent): void {
  openAt(event);
}

function toggleExpanded(): void {
  if (props.project.id !== undefined) {
    uiState.setProjectCollapsed(props.project.workspace_id, props.project.id, expanded.value);
  }
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
       :icon="expanded ? 'layers-3' : 'layers'"
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
        :workspace-id="project.workspace_id"
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
        suppress-refreshing
        @retry="loadTree"
      />
    </template>
    <ConfirmDialog
      :open="deleteProjectOpen"
      tone="danger"
      title="Delete project?"
      message="This moves the project and its contents to Trash. It can be restored by an administrator."
      :detail="project.name"
      detail-icon="layers"
      confirm-label="Delete project"
      confirm-icon="trash-2"
       @confirm="deleteProject"
      @cancel="deleteProjectOpen = false"
    />

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
