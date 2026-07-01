<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { onBeforeRouteLeave, onBeforeRouteUpdate, useRoute, useRouter } from 'vue-router';
import BacklinksPanel from '@/components/notas/BacklinksPanel.vue';
import CasConflictView from '@/components/notas/CasConflictView.vue';
import HistoryPanel from '@/components/notas/HistoryPanel.vue';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import NoteEditor from '@/components/notas/NoteEditor.vue';
import PropertiesEditor from '@/components/notas/PropertiesEditor.vue';
import PropertiesPanel from '@/components/notas/PropertiesPanel.vue';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import WikiLinkSuggest from '@/components/notas/WikiLinkSuggest.vue';
import SharePanel from '@/components/share/SharePanel.vue';
import EditorToolbar from '@/components/shell/EditorToolbar.vue';
import Icon from '@/components/ui/Icon.vue';
import TabStrip, { type Tab } from '@/components/ui/TabStrip.vue';
import { useBreakpoint } from '@/composables/useBreakpoint';
import type { MergeSegment } from '@/composables/useCasMerge';
import { useCasMerge } from '@/composables/useCasMerge';
import { useMarkdownDoc } from '@/composables/useMarkdownDoc';
import { useWikilinkTitles } from '@/composables/useWikilinkTitles';
import { joinFrontmatter, splitFrontmatter } from '@/lib/frontmatter';
import { type WikilinkRef, wikilinkHref } from '@/lib/wikilink';
import { useDocumentsStore } from '@/stores/documents';
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
const { load, save } = useMarkdownDoc();
const { merge } = useCasMerge();
const { isMobile } = useBreakpoint();

function goBackToTree(): void {
  void router.push({ name: 'notes' });
}

const editorRef = ref<InstanceType<typeof NoteEditor> | null>(null);
const suggestRef = ref<InstanceType<typeof WikiLinkSuggest> | null>(null);
const sidebarRef = ref<InstanceType<typeof NotesSidebar> | null>(null);

const slug = computed(() => {
  const s = route.params.slug;
  return typeof s === 'string' && s.length > 0 ? s : null;
});

const ws = computed(() => workspace.activeWorkspaceSlug ?? '');

const title = ref('');
const body = ref('');
const meta = ref<Record<string, unknown>>({});
const headRevisionId = ref('');
const dirty = ref(false);

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
const loadError = ref<string | null>(null);
const wikilinkQuery = ref<string | null>(null);
const wikilinkCaret = ref<{ left: number; top: number } | null>(null);

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

async function loadDoc(): Promise<void> {
  loadError.value = null;
  conflictOpen.value = false;
  conflictSegments.value = [];

  if (slug.value === null || ws.value === '') {
    body.value = '';
    title.value = '';
    meta.value = {};
    baseContent.value = '';
    return;
  }

  try {
    const result = await load(ws.value, slug.value);

    // A uuid-addressed URL (from search, a wikilink, etc.) is canonicalized to
    // the pretty slug; the watch re-runs loadDoc with the slug and proceeds.
    if (result.slug !== null && result.slug !== slug.value) {
      await router.replace({ name: 'notes', params: { slug: result.slug } });
      return;
    }

    body.value = result.body;
    meta.value = result.meta;
    headRevisionId.value = result.headRevisionId;
    baseContent.value = joinFrontmatter(result.meta, result.body);
    title.value = typeof result.meta.title === 'string' ? result.meta.title : (slug.value ?? '');
    dirty.value = false;
    tabsStore.open(ws.value, slug.value, title.value);
    await documents.loadBacklinks(ws.value, slug.value);
  } catch (e) {
    // The note is gone (deleted here, elsewhere, or a stale persisted tab): drop
    // its tab and move on instead of stranding the user on a broken tab.
    const status = (e as { status?: number }).status ?? 0;
    if (status === 404 && slug.value !== null && ws.value !== '') {
      const next = tabsStore.close(ws.value, slug.value);
      void router.replace(next !== null ? { name: 'notes', params: { slug: next } } : { name: 'notes' });
      return;
    }
    loadError.value = e instanceof Error ? e.message : 'Failed to load document';
  }
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
function flushPendingSave(): void {
  if (saveTimer === null) return;
  clearTimeout(saveTimer);
  saveTimer = null;

  const targetSlug = slug.value;
  if (targetSlug === null || ws.value === '') return;

  const currentBody = editorRef.value?.currentMarkdown() ?? body.value;
  void save(ws.value, targetSlug, currentBody, meta.value, headRevisionId.value);
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

function onWikilinkQuery(query: string | null, caret: { left: number; top: number } | null): void {
  wikilinkQuery.value = query;
  wikilinkCaret.value = caret;
}

function onSuggestSelect(ref: WikilinkRef): void {
  editorRef.value?.insertWikilink(ref);
  wikilinkQuery.value = null;
  wikilinkCaret.value = null;
}

function onEditorKeydown(event: KeyboardEvent): void {
  if (suggestRef.value?.open !== true) return;

  if (event.key === 'ArrowDown') {
    event.preventDefault();
    suggestRef.value.moveDown();
  } else if (event.key === 'ArrowUp') {
    event.preventDefault();
    suggestRef.value.moveUp();
  } else if (event.key === 'Enter') {
    event.preventDefault();
    suggestRef.value.confirmActive();
  } else if (event.key === 'Escape') {
    wikilinkQuery.value = null;
    wikilinkCaret.value = null;
  }
}

// Flush before the outgoing document is replaced: update fires on a note→note
// slug change, leave fires when navigating out of Notes. Both run before the
// route (and `slug`) updates, so the pending save still targets this document.
onBeforeRouteUpdate(() => {
  flushPendingSave();
});
onBeforeRouteLeave(() => {
  flushPendingSave();
});

watch([slug, ws], loadDoc, { immediate: true });

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
      v-if="isMobile && slug"
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
      <template v-if="slug">
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

    <div class="flex-1 overflow-y-auto">
      <div
        :style="{
          maxWidth: isMobile || ui.editorWide ? 'none' : '980px',
          margin: '0 auto',
          padding: isMobile ? '16px 16px 32px' : '30px 40px',
          position: 'relative',
        }"
      >
        <p
          v-if="loadError"
          style="
            padding: 8px 12px;
            border-radius: var(--r-md);
            background: var(--c-banner-err-bg);
            color: var(--c-banner-err-fg);
            font-size: var(--fs-sm);
          "
        >
          {{ loadError }}
        </p>

        <template v-if="slug">
          <h1
            style="font-size: 22px; font-weight: var(--fw-bold); letter-spacing: -0.01em; color: var(--c-foreground); margin-bottom: 14px;"
          >
            {{ title || 'Untitled' }}
          </h1>

          <PropertiesEditor :ws="ws" :meta="meta" @change="onMetaChange" />

          <div @keydown="onEditorKeydown">
            <NoteEditor
              ref="editorRef"
              v-model:mode="editorMode"
              v-model:reading="editorReading"
              :body="body"
              :wikilink-titles="wikilinkTitles"
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

        <p
          v-else
          style="font-size: var(--fs-sm); color: var(--c-muted);"
        >
          Select a document from the tree to start editing.
        </p>
      </div>
    </div>

    <template #inspector-properties>
      <PropertiesPanel :meta="meta" />
    </template>

    <template #inspector-backlinks>
      <BacklinksPanel
        :backlinks="documents.backlinks"
        @navigate="(s) => router.push({ name: 'notes', params: { slug: s } })"
      />
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
