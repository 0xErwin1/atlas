<script lang="ts">
export type NoteTarget = {
  workspaceSlug: string;
  slug: string;
};

type NoteResourceStatus = 'idle' | 'pending' | 'ready' | 'error';

export type NoteResourceState = {
  target: NoteTarget | null;
  sequence: number;
  status: NoteResourceStatus;
  hasContent: boolean;
  error: string | null;
};

type NoteResourceLoadResult<T> = { accepted: true; value: T } | { accepted: false; error?: unknown };

export function createNoteResourceState(): NoteResourceState {
  return {
    target: null,
    sequence: 0,
    status: 'idle',
    hasContent: false,
    error: null,
  };
}

function targetsEqual(left: NoteTarget | null, right: NoteTarget): boolean {
  return left?.workspaceSlug === right.workspaceSlug && left.slug === right.slug;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : 'Failed to load document';
}

function startNoteResourceTransition(state: NoteResourceState, target: NoteTarget): number {
  const targetChanged = !targetsEqual(state.target, target);
  const sequence = state.sequence + 1;

  state.target = target;
  state.sequence = sequence;
  state.status = 'pending';
  state.error = null;
  if (targetChanged) state.hasContent = false;

  return sequence;
}

function acceptsNoteResourceLoad(state: NoteResourceState, target: NoteTarget, sequence: number): boolean {
  return state.sequence === sequence && targetsEqual(state.target, target);
}

function settleNoteResourceError(
  state: NoteResourceState,
  target: NoteTarget,
  sequence: number,
  error: unknown,
): boolean {
  if (!acceptsNoteResourceLoad(state, target, sequence)) return false;

  state.status = 'error';
  state.hasContent = false;
  state.error = errorMessage(error);
  return true;
}

async function runRegisteredNoteResourceLoad<T>(
  state: NoteResourceState,
  target: NoteTarget,
  sequence: number,
  load: () => Promise<T>,
): Promise<NoteResourceLoadResult<T>> {
  if (!acceptsNoteResourceLoad(state, target, sequence)) return { accepted: false };

  try {
    const value = await load();
    if (!acceptsNoteResourceLoad(state, target, sequence)) return { accepted: false };

    state.status = 'ready';
    state.hasContent = true;
    return { accepted: true, value };
  } catch (error) {
    return settleNoteResourceError(state, target, sequence, error)
      ? { accepted: false, error }
      : { accepted: false };
  }
}

export async function runNoteResourceLoad<T>(
  state: NoteResourceState,
  target: NoteTarget,
  load: () => Promise<T>,
): Promise<NoteResourceLoadResult<T>> {
  return runRegisteredNoteResourceLoad(state, target, startNoteResourceTransition(state, target), load);
}

export async function flushThenLoadNoteResource<T>(
  state: NoteResourceState,
  target: NoteTarget,
  flush: () => Promise<void>,
  load: () => Promise<T>,
): Promise<NoteResourceLoadResult<T>> {
  const sequence = startNoteResourceTransition(state, target);
  try {
    await flush();
  } catch (error) {
    return settleNoteResourceError(state, target, sequence, error)
      ? { accepted: false, error }
      : { accepted: false };
  }

  return runRegisteredNoteResourceLoad(state, target, sequence, load);
}

export type ReconcileDecision = 'ignore' | 'apply' | 'keep-and-flag';

/**
 * Decides how a realtime `document.updated`/resync signal should reconcile
 * against the currently open note. Dirty local edits are never overwritten —
 * they are flagged instead, so the next save runs through the existing CAS
 * conflict flow rather than silently losing unsaved work.
 */
export function planDocumentReconcile(
  openDocumentId: string | null,
  eventDocumentId: string | null,
  dirty: boolean,
): ReconcileDecision {
  if (openDocumentId === null || eventDocumentId === null || eventDocumentId !== openDocumentId) {
    return 'ignore';
  }

  return dirty ? 'keep-and-flag' : 'apply';
}

export function isDocumentMissingError(error: unknown): boolean {
  return (error as { status?: number } | undefined)?.status === 404;
}
</script>

<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue';
import { onBeforeRouteLeave, onBeforeRouteUpdate, useRoute, useRouter } from 'vue-router';
import BacklinksPanel from '@/components/notas/BacklinksPanel.vue';
import CasConflictView from '@/components/notas/CasConflictView.vue';
import DocumentComments from '@/components/notas/DocumentComments.vue';
import HistoryPanel from '@/components/notas/HistoryPanel.vue';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import NoteEditor from '@/components/notas/NoteEditor.vue';
import PropertiesEditor from '@/components/notas/PropertiesEditor.vue';
import PropertiesPanel from '@/components/notas/PropertiesPanel.vue';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import WikiLinkSuggest from '@/components/notas/WikiLinkSuggest.vue';
import SharePanel from '@/components/share/SharePanel.vue';
import EditorToolbar from '@/components/shell/EditorToolbar.vue';
import EmptyState from '@/components/states/EmptyState.vue';
import ErrorState from '@/components/states/ErrorState.vue';
import LoadingState from '@/components/states/LoadingState.vue';
import Icon from '@/components/ui/Icon.vue';
import PresenceAvatars from '@/components/ui/PresenceAvatars.vue';
import TabStrip, { type Tab } from '@/components/ui/TabStrip.vue';
import { useBreakpoint } from '@/composables/useBreakpoint';
import type { MergeSegment } from '@/composables/useCasMerge';
import { useCasMerge } from '@/composables/useCasMerge';
import { useDocumentPresence } from '@/composables/useDocumentPresence';
import { useLiveUpdates } from '@/composables/useLiveUpdates';
import type { LoadResult } from '@/composables/useMarkdownDoc';
import { useMarkdownDoc } from '@/composables/useMarkdownDoc';
import { useWikilinkSuggest } from '@/composables/useWikilinkSuggest';
import { useWikilinkTitles } from '@/composables/useWikilinkTitles';
import { EVENT_TYPE, PRESENCE_UPDATED } from '@/lib/eventTypes';
import { joinFrontmatter, splitFrontmatter } from '@/lib/frontmatter';
import { type WikilinkRef, wikilinkHref } from '@/lib/wikilink';
import { useDocumentsStore } from '@/stores/documents';
import { useLastViewedStore } from '@/stores/lastViewed';
import { useNotesTabsStore } from '@/stores/notesTabs';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';
import AppShell from '@/views/AppShell.vue';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import NotesSidebar from '@/views/NotesSidebar.vue';

const route = useRoute();
const router = useRouter();
const workspace = useWorkspaceStore();
const documents = useDocumentsStore();
const ui = useUiStore();
const tabsStore = useNotesTabsStore();
const lastViewed = useLastViewedStore();
const { load, save } = useMarkdownDoc();
const { merge } = useCasMerge();
const { isMobile } = useBreakpoint();

function goBackToTree(): void {
  void router.push({ name: 'notes' });
}

/**
 * Uploads a pasted/dropped image as an attachment of the open note and returns
 * the same-origin URL to embed. The URL authenticates via the session cookie, so
 * the inserted `![](…)` renders directly in the live preview with no blob step.
 */
async function onUploadImage(file: File): Promise<string | null> {
  if (ws.value === '' || slug.value === null) return null;

  const attachment = await documents.uploadAttachment(ws.value, slug.value, file);
  if (attachment === null) {
    ui.showBanner(documents.error ?? 'Failed to upload image', 'error');
    return null;
  }

  return `/api/workspaces/${ws.value}/attachments/${attachment.id}`;
}

const editorRef = ref<InstanceType<typeof NoteEditor> | null>(null);
const suggestRef = ref<InstanceType<typeof WikiLinkSuggest> | null>(null);
const sidebarRef = ref<InstanceType<typeof NotesSidebar> | null>(null);
// The scrollable note surface (title + properties + editor). It is not remounted
// between notes, so its scroll offset must be reset on a switch.
const scrollAreaRef = ref<HTMLElement | null>(null);

const slug = computed(() => {
  const s = route.params.slug;
  return typeof s === 'string' && s.length > 0 ? s : null;
});

const ws = computed(() => workspace.activeWorkspaceSlug ?? '');

// Live document presence: heartbeat the viewer into the open note and surface who
// else is editing it. `document.updated` for the open note and every resync
// (reconnect or explicit `resync` marker) reconcile the open note via CAS,
// scoped to it by `documentId` so other notes' edits never touch this buffer.
const presence = useDocumentPresence(ws, slug);
useLiveUpdates(ws, {
  onEvent: (evt) => {
    if (evt.type === PRESENCE_UPDATED) {
      presence.apply(evt.envelope);
      return;
    }
    if (evt.type === EVENT_TYPE.DOCUMENT_UPDATED) void reconcileOpenNote(evt.envelope.document_id ?? null);
  },
  onResync: () => void reconcileOpenNote(documentId.value),
});

const title = ref('');
const body = ref('');
const meta = ref<Record<string, unknown>>({});
const headRevisionId = ref('');
const dirty = ref(false);
// The open note's stable id, used to scope realtime reconcile to this note.
const documentId = ref<string | null>(null);
// Set when a remote edit arrived while the open note had unsaved local edits;
// cleared by the next successful save or a clean reconcile.
const remoteChangesPending = ref(false);

// Editor view mode, owned here so the toolbar's segmented control drives the
// shared editor (which renders no in-body controls for Notes).
const editorMode = ref<'live' | 'source'>('live');
const editorReading = ref(false);

function toggleEditorSource(): void {
  editorMode.value = editorMode.value === 'source' ? 'live' : 'source';
}

function toggleEditorReading(): void {
  editorReading.value = !editorReading.value;
}
const noteResource = ref(createNoteResourceState());
const hasDocumentContent = computed(() => noteResource.value.hasContent);

// `[[wikilink]]` autocomplete glue, shared with the task description editor.
const {
  query: wikilinkQuery,
  caret: wikilinkCaret,
  onQuery: onWikilinkQuery,
  onSelect: onSuggestSelect,
  onKeydown: onEditorKeydown,
} = useWikilinkSuggest(
  () => editorRef.value,
  () => suggestRef.value,
);

// Resolves id-bound wikilinks' current titles so rendered links track renames.
const wikilinkTitles = useWikilinkTitles(ws, body);

// The full document content (frontmatter + body) as loaded at headRevisionId.
// It is the 3-way merge BASE; never mutated by local edits.
const baseContent = ref('');

const conflictOpen = ref(false);
const conflictSegments = ref<MergeSegment[]>([]);

const breadcrumbs = computed(() => {
  const docTitle = title.value || 'Untitled';
  const parent =
    typeof meta.value.project === 'string'
      ? meta.value.project
      : typeof meta.value.folder === 'string'
        ? meta.value.folder
        : null;

  return parent !== null && parent !== '' ? ['Atlas', parent, docTitle] : ['Atlas', docTitle];
});

const editorTabs = computed<Tab[]>(() =>
  tabsStore.tabs(ws.value).map((t) => ({
    id: t.slug,
    name: t.title || 'Untitled',
    icon: 'file',
    active: t.slug === slug.value,
    dirty: t.slug === slug.value && dirty.value,
  })),
);

function onSelectTab(id: string): void {
  if (id !== slug.value) void router.push({ name: 'notes', params: { slug: id } });
}

function onCloseTab(id: string): void {
  const nextSlug = tabsStore.close(ws.value, id);
  if (id !== slug.value) return;

  void router.push(nextSlug !== null ? { name: 'notes', params: { slug: nextSlug } } : { name: 'notes' });
}

// After a bulk close, navigate only when the active note was among those closed;
// `anchor` is the note to fall back to (null = the notes root).
function navigateAfterClose(anchor: string | null): void {
  if (slug.value === null) return;
  if (tabsStore.tabs(ws.value).some((t) => t.slug === slug.value)) return;
  void router.push(anchor !== null ? { name: 'notes', params: { slug: anchor } } : { name: 'notes' });
}

function onCloseOthers(id: string): void {
  navigateAfterClose(tabsStore.closeOthers(ws.value, id));
}

function onCloseRight(id: string): void {
  navigateAfterClose(tabsStore.closeRight(ws.value, id));
}

function onCloseAll(): void {
  navigateAfterClose(tabsStore.closeAll(ws.value));
}

let saveTimer: ReturnType<typeof setTimeout> | null = null;

function clearDocument(): void {
  body.value = '';
  title.value = '';
  meta.value = {};
  headRevisionId.value = '';
  baseContent.value = '';
  dirty.value = false;
  documentId.value = null;
  remoteChangesPending.value = false;
}

/**
 * Applies a loaded document to the editor state. Shared by the route-driven
 * `loadDoc` and the realtime reconcile path, so both advance the CAS base
 * (`baseContent`/`headRevisionId`) through the exact same assignments.
 */
function applyLoadedDocument(result: LoadResult, fallbackTitle: string): void {
  body.value = result.body;
  meta.value = result.meta;
  headRevisionId.value = result.headRevisionId;
  baseContent.value = joinFrontmatter(result.meta, result.body);
  title.value = typeof result.meta.title === 'string' ? result.meta.title : fallbackTitle;
  dirty.value = false;
  documentId.value = result.id;
}

/**
 * Recovers from a document that no longer exists (404): drops the stale
 * last-viewed pointer and the tab, then routes to the next open tab or the
 * notes root. Shared by `loadDoc` and the realtime reconcile path.
 */
function handleMissingDocument(target: NoteTarget): void {
  lastViewed.clearIfMatches(target.workspaceSlug, {
    name: 'notes',
    params: { slug: target.slug },
  });
  const next = tabsStore.close(target.workspaceSlug, target.slug) ?? tabsStore.tabs(target.workspaceSlug)[0]?.slug ?? null;
  void router.replace(next !== null ? { name: 'notes', params: { slug: next } } : { name: 'notes' });
}

/**
 * Reconciles the open note against a realtime signal (a `document.updated`
 * event scoped to this note, or a full resync). A clean note advances its CAS
 * base to the remote revision; a dirty note is left untouched and flagged so
 * the next save runs through the existing CAS conflict/merge flow instead of
 * silently losing unsaved edits.
 */
async function reconcileOpenNote(eventDocumentId: string | null): Promise<void> {
  if (slug.value === null || ws.value === '') return;

  const decision = planDocumentReconcile(documentId.value, eventDocumentId, dirty.value);
  if (decision === 'ignore') return;

  if (decision === 'keep-and-flag') {
    remoteChangesPending.value = true;
    return;
  }

  const target: NoteTarget = { workspaceSlug: ws.value, slug: slug.value };
  try {
    const result = await load(target.workspaceSlug, target.slug);
    applyLoadedDocument(result, target.slug);
    remoteChangesPending.value = false;
  } catch (error) {
    if (isDocumentMissingError(error)) handleMissingDocument(target);
  }
}

async function loadDoc(target: NoteTarget | null, previousTarget: NoteTarget | null): Promise<void> {
  conflictOpen.value = false;
  conflictSegments.value = [];

  if (target === null) {
    noteResource.value = createNoteResourceState();
    documents.clearSecondaryTarget();
    clearDocument();
    return;
  }

  documents.resetSecondaryTarget(target.workspaceSlug, target.slug);

  const targetChanged =
    previousTarget !== null &&
    (previousTarget.workspaceSlug !== target.workspaceSlug || previousTarget.slug !== target.slug);
  const shouldFlush = targetChanged && saveTimer !== null;
  const loadResultPromise = shouldFlush
    ? flushThenLoadNoteResource(noteResource.value, target, () => flushPendingSave(previousTarget), () =>
        load(target.workspaceSlug, target.slug),
      )
    : runNoteResourceLoad(noteResource.value, target, () => load(target.workspaceSlug, target.slug));

  if (!noteResource.value.hasContent) clearDocument();
  const loadResult = await loadResultPromise;
  if (!loadResult.accepted) {
    if (isDocumentMissingError(loadResult.error)) handleMissingDocument(target);
    return;
  }

  const result = loadResult.value;
  if (result.slug !== null && result.slug !== target.slug) {
    await router.replace({ name: 'notes', params: { slug: result.slug } });
    return;
  }

  applyLoadedDocument(result, target.slug);
  tabsStore.open(target.workspaceSlug, target.slug, title.value);
  await documents.loadBacklinks(target.workspaceSlug, target.slug);
}

async function persist(): Promise<void> {
  if (slug.value === null || ws.value === '') return;

  const currentBody = editorRef.value?.currentMarkdown() ?? body.value;
  const result = await save(ws.value, slug.value, currentBody, meta.value, headRevisionId.value);

  if (result.kind === 'ok') {
    onSaved(joinFrontmatter(meta.value, currentBody), result.headRevisionId);
    return;
  }

  if (result.kind === 'error') {
    ui.showBanner(result.hint ?? result.title, 'error');
    return;
  }

  // CAS conflict: run the 3-way merge against the loaded base, never overwrite.
  const mine = joinFrontmatter(meta.value, currentBody);
  const merged = merge({
    base: baseContent.value,
    mine,
    patch: result.problem.base_to_current_patch,
  });

  if (merged.kind === 'clean') {
    await resave(merged.merged, result.problem.current_revision_id, true);
    return;
  }

  // Overlapping edits: open the focused conflict view (never last-write-wins).
  conflictSegments.value = merged.segments;
  conflictOpen.value = true;
  // Stash the server revision the resolution must be saved against.
  pendingConflictRevision.value = result.problem.current_revision_id;
}

const pendingConflictRevision = ref('');

async function resave(content: string, baseRevisionId: string, autoMerged: boolean): Promise<void> {
  if (slug.value === null || ws.value === '') return;

  const { body: resolvedBody, meta: resolvedMeta } = splitFrontmatter(content);
  const result = await save(ws.value, slug.value, resolvedBody, resolvedMeta, baseRevisionId);

  if (result.kind === 'ok') {
    meta.value = resolvedMeta;
    body.value = resolvedBody;
    title.value = typeof resolvedMeta.title === 'string' ? resolvedMeta.title : title.value;
    onSaved(content, result.headRevisionId);
    ui.showBanner(autoMerged ? 'Conflict auto-merged and saved.' : 'Conflict resolved and saved.', 'success');
    return;
  }

  if (result.kind === 'conflict') {
    // The document moved again between merge and resave: re-enter the flow.
    const mine = joinFrontmatter(resolvedMeta, resolvedBody);
    const merged = merge({
      base: baseContent.value,
      mine,
      patch: result.problem.base_to_current_patch,
    });
    if (merged.kind === 'clean') {
      await resave(merged.merged, result.problem.current_revision_id, true);
      return;
    }
    conflictSegments.value = merged.segments;
    conflictOpen.value = true;
    pendingConflictRevision.value = result.problem.current_revision_id;
    return;
  }

  ui.showBanner(result.hint ?? result.title, 'error');
}

function onSaved(content: string, revisionId: string): void {
  dirty.value = false;
  conflictOpen.value = false;
  conflictSegments.value = [];
  baseContent.value = content;
  if (revisionId !== '') headRevisionId.value = revisionId;
  remoteChangesPending.value = false;
}

async function onConflictResolve(content: string): Promise<void> {
  conflictOpen.value = false;
  await resave(content, pendingConflictRevision.value, false);
}

function onConflictCancel(): void {
  conflictOpen.value = false;
  conflictSegments.value = [];
  ui.showBanner('Conflict not resolved — your local edits are kept unsaved.', 'warning');
}

function onChange(markdown: string): void {
  body.value = markdown;
  dirty.value = true;

  if (saveTimer !== null) clearTimeout(saveTimer);
  saveTimer = setTimeout(() => void persist(), 800);
}

/**
 * Persist a pending debounced edit before the current document goes away
 * (switching notes or leaving the view), so the last keystrokes within the
 * debounce window are never dropped. Runs from the route guards, which fire
 * while the refs still point at the outgoing document.
 *
 * This is best-effort: the identity is captured as `save` arguments and the
 * component refs are intentionally not updated afterwards, because the next
 * document's own load owns them. A CAS conflict here is left unsaved (the same
 * outcome as before), rather than merged against now-stale local state.
 */
async function flushPendingSave(target: NoteTarget | null = null): Promise<void> {
  if (saveTimer === null) return;
  clearTimeout(saveTimer);
  saveTimer = null;

  const saveTarget = target ?? (slug.value === null || ws.value === '' ? null : { workspaceSlug: ws.value, slug: slug.value });
  if (saveTarget === null) return;

  const currentBody = editorRef.value?.currentMarkdown() ?? body.value;
  await save(saveTarget.workspaceSlug, saveTarget.slug, currentBody, meta.value, headRevisionId.value);
}

function onMetaChange(newMeta: Record<string, unknown>): void {
  meta.value = newMeta;
  title.value = typeof newMeta.title === 'string' ? newMeta.title : (slug.value ?? '');
  dirty.value = true;

  if (saveTimer !== null) clearTimeout(saveTimer);
  saveTimer = setTimeout(() => void persist(), 800);
}

function onNavigateWikilink(ref: WikilinkRef): void {
  void router.push(wikilinkHref(ref));
}

// Flush before the outgoing document is replaced: update fires on a note→note
// slug change, leave fires when navigating out of Notes. Both run before the
// route (and `slug`) updates, so the pending save still targets this document.
onBeforeRouteUpdate(async () => {
  await flushPendingSave();
});
onBeforeRouteLeave(async () => {
  await flushPendingSave();
});

watch(
  [slug, ws],
  ([nextSlug, nextWorkspace], [previousSlug, previousWorkspace]) => {
    const target =
      typeof nextSlug !== 'string' || typeof nextWorkspace !== 'string' || nextWorkspace === ''
        ? null
        : { workspaceSlug: nextWorkspace, slug: nextSlug };
    const previousTarget =
      typeof previousSlug !== 'string' || typeof previousWorkspace !== 'string' || previousWorkspace === ''
        ? null
        : { workspaceSlug: previousWorkspace, slug: previousSlug };
    void loadDoc(target, previousTarget);
  },
  { immediate: true },
);

// A new note opens at the top: the scroll surface persists across switches, so
// without this it would keep the previous note's scroll offset.
watch(slug, () => {
  void nextTick(() => {
    if (scrollAreaRef.value !== null) scrollAreaRef.value.scrollTop = 0;
  });
});

watch(title, (t) => {
  if (slug.value !== null && ws.value !== '') tabsStore.setTitle(ws.value, slug.value, t);
});
</script>

<template>
  <AppShell sidebar-title="Notes" sidebar-icon="file-text" :mobile-detail="slug !== null">
    <template #sidebar-actions>
      <button type="button" class="atl-gbtn" title="Search ⌘K" aria-label="Search" @click="ui.openPalette()">
        <Icon name="search" :size="14" />
      </button>
      <button
        type="button"
        class="atl-gbtn"
        title="Collapse sidebar"
        aria-label="Collapse sidebar"
        @click="ui.toggleSidebar()"
      >
        <Icon name="panel-left" :size="13" />
      </button>
    </template>

    <template #sidebar>
      <NotesSidebar ref="sidebarRef" />
    </template>

    <template #sidebar-footer>
      <button
        type="button"
        class="atl-gbtn"
        style="width: 100%; justify-content: flex-start; height: 26px; gap: 7px; color: var(--c-foreground);"
        @click="sidebarRef?.openNewPage()"
      >
        <Icon name="plus" :size="14" />
        New page
      </button>
    </template>

    <div
      v-if="isMobile && slug && hasDocumentContent"
      class="flex items-center"
      style="height: 44px; flex: 0 0 44px; padding: 0 6px; gap: 4px; border-bottom: 1px solid var(--c-border);"
    >
      <button type="button" class="atl-gbtn" title="Back" aria-label="Back to notes" @click="goBackToTree">
        <Icon name="chevron-left" :size="20" />
      </button>
      <span class="flex-1 truncate" style="font-size: var(--fs-lg); font-weight: var(--fw-bold); color: var(--c-foreground);">
        {{ title || 'Untitled' }}
      </span>
      <button
        type="button"
        title="Markdown source"
        aria-label="Markdown source"
        class="atl-gbtn"
        :class="{ on: editorMode === 'source' }"
        :aria-pressed="editorMode === 'source'"
        style="width: 28px; height: 28px;"
        @click="toggleEditorSource"
      >
        <Icon name="code" :size="15" />
      </button>
      <button
        type="button"
        title="Rendered view"
        aria-label="Rendered view"
        class="atl-gbtn"
        :class="{ on: editorReading }"
        :aria-pressed="editorReading"
        style="width: 28px; height: 28px;"
        @click="toggleEditorReading"
      >
        <Icon name="pencil" :size="15" />
      </button>
      <button
        type="button"
        title="Share"
        aria-label="Share"
        class="atl-gbtn"
        style="width: 28px; height: 28px;"
        @click="ui.openShare(`${title || 'Document'} · note`)"
      >
        <Icon name="user" :size="15" />
      </button>
      <button
        type="button"
        title="Details"
        aria-label="Details"
        class="atl-gbtn"
        :class="{ on: ui.inspectorOpen }"
        style="width: 28px; height: 28px;"
        @click="ui.toggleInspector()"
      >
        <Icon name="panel-right" :size="15" />
      </button>
    </div>

    <TabStrip
      v-if="!isMobile && editorTabs.length > 0"
      :tabs="editorTabs"
      closable
      @select="onSelectTab"
      @close="onCloseTab"
      @close-others="onCloseOthers"
      @close-right="onCloseRight"
      @close-all="onCloseAll"
    >
      <template #right>
        <button
          type="button"
          class="atl-gbtn"
          title="New page"
          aria-label="New page"
          @click="sidebarRef?.openNewPage()"
        >
          <Icon name="plus" :size="13" />
        </button>
        <button
          type="button"
          class="atl-gbtn"
          title="Command palette ⌘K"
          aria-label="Command palette"
          @click="ui.openPalette()"
        >
          <Icon name="command" :size="13" />
        </button>
      </template>
    </TabStrip>

    <EditorToolbar v-if="!isMobile" :breadcrumbs="breadcrumbs" :dirty="dirty">
      <template v-if="slug && hasDocumentContent">
        <div class="atl-seg" role="group" aria-label="Editor view mode">
          <button
            type="button"
            class="atl-segb accent"
            :class="{ on: ui.editorWide }"
            :title="ui.editorWide ? 'Readable width' : 'Wider text'"
            :aria-pressed="ui.editorWide"
            @click="ui.toggleEditorWide()"
          >
            <Icon name="widen" :size="14" />
          </button>
          <div aria-hidden="true" style="width: 1px; height: 14px; background: var(--c-border); margin: 0 1px;" />
          <button
            type="button"
            class="atl-segb"
            :class="{ on: editorMode === 'source' }"
            title="Markdown source"
            :aria-pressed="editorMode === 'source'"
            @click="toggleEditorSource"
          >
            <Icon name="code" :size="14" />
          </button>
          <button
            type="button"
            class="atl-segb"
            :class="{ on: editorReading }"
            title="Rendered view"
            :aria-pressed="editorReading"
            @click="toggleEditorReading"
          >
            <Icon name="pencil" :size="14" />
          </button>
        </div>

        <div aria-hidden="true" style="width: 1px; height: 18px; background: var(--c-border);" />
      </template>

      <button
        v-if="slug && hasDocumentContent && remoteChangesPending"
        type="button"
        title="Remote changes pending — click to reconcile"
        aria-label="Remote changes pending"
        class="atl-gbtn on"
        style="width: 28px; height: 28px;"
        @click="reconcileOpenNote(documentId)"
      >
        <Icon name="refresh-cw" :size="15" />
      </button>

      <PresenceAvatars v-if="slug && hasDocumentContent" :actors="presence.actors" />

      <button
        type="button"
        title="Toggle inspector"
        aria-label="Toggle inspector"
        class="atl-gbtn"
        :class="{ on: ui.inspectorOpen }"
        style="width: 28px; height: 28px;"
        @click="ui.toggleInspector()"
      >
        <Icon name="panel-right" :size="15" />
      </button>
    </EditorToolbar>

    <div ref="scrollAreaRef" class="flex-1 overflow-y-auto">
      <div
        :style="{
          maxWidth: isMobile || ui.editorWide ? 'none' : '980px',
          margin: '0 auto',
          padding: isMobile ? '16px 16px 32px' : '30px 40px',
          position: 'relative',
        }"
      >
        <ErrorState
          v-if="noteResource.status === 'error'"
          title="Couldn’t load note"
          :hint="noteResource.error ?? 'Failed to load document'"
          @retry="loadDoc(noteResource.target, null)"
        />

        <LoadingState
          v-else-if="noteResource.status === 'pending' && !hasDocumentContent"
          label="Loading note…"
        />

        <template v-else-if="slug && hasDocumentContent">
          <h1
            style="font-size: 22px; font-weight: var(--fw-bold); letter-spacing: -0.01em; color: var(--c-foreground); margin-bottom: 14px;"
          >
            {{ title || 'Untitled' }}
          </h1>

          <PropertiesEditor :ws="ws" :meta="meta" @change="onMetaChange" />

          <div @keydown="onEditorKeydown">
            <NoteEditor
              ref="editorRef"
              :key="slug"
              v-model:mode="editorMode"
              v-model:reading="editorReading"
              :body="body"
              :wikilink-titles="wikilinkTitles"
              :upload-image="onUploadImage"
              @change="onChange"
              @navigate-wikilink="onNavigateWikilink"
              @wikilink-query="onWikilinkQuery"
            />

            <div
              v-if="wikilinkCaret"
              :style="{
                position: 'fixed',
                left: `${wikilinkCaret.left}px`,
                top: `${wikilinkCaret.top}px`,
                zIndex: 40,
              }"
            >
              <WikiLinkSuggest
                ref="suggestRef"
                :ws="ws"
                :query="wikilinkQuery"
                @select="onSuggestSelect"
              />
            </div>
          </div>
        </template>

        <EmptyState
          v-else
          title="No document open"
          hint="Select a document from the tree to start editing."
        />
      </div>
    </div>

    <template #inspector-properties>
      <PropertiesPanel :meta="meta" />
    </template>

    <template #inspector-backlinks>
      <BacklinksPanel
        :backlinks="documents.backlinks"
        :status="documents.backlinksStatus"
        :error="documents.backlinksError"
        @navigate="(s) => router.push({ name: 'notes', params: { slug: s } })"
        @retry="slug && documents.loadBacklinks(ws, slug)"
      />
    </template>

    <template #inspector-comments>
      <DocumentComments v-if="slug" :ws="ws" :slug="slug" />
      <p v-else style="font-size: var(--fs-sm); color: var(--c-muted);">
        Open a document to see its comments.
      </p>
    </template>

    <template #inspector-activity>
      <HistoryPanel />
    </template>

    <template #inspector-share>
      <SharePanel :resource-label="`${title || 'Document'} · note`" />
    </template>

    <CasConflictView
      :open="conflictOpen"
      :segments="conflictSegments"
      @resolve="onConflictResolve"
      @cancel="onConflictCancel"
    />
  </AppShell>
</template>
