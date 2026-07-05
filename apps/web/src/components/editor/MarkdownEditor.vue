<script setup lang="ts">
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { languages } from '@codemirror/language-data';
import { Compartment, EditorState } from '@codemirror/state';
import { EditorView, keymap } from '@codemirror/view';
import { GFM } from '@lezer/markdown';
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import Icon from '@/components/ui/Icon.vue';
import { restoreSelection, snapshotSelection } from '@/lib/editorSelection';
import { filesFromClipboard, filesFromDataTransfer, isImageFile } from '@/lib/fileTransfer';
import {
  detectWikilinkTrigger,
  formatWikilink,
  type WikilinkRef,
  type WikilinkTrigger,
} from '@/lib/wikilink';
import { useUiStore } from '@/stores/ui';
import { atlasHighlight } from './highlight';
import { livePreview } from './livePreviewExtension';
import { atlasMarkdownTheme } from './theme';

/**
 * Shared Obsidian-style "Live Preview" markdown editor built on CodeMirror 6.
 *
 * The CodeMirror document IS the markdown source of truth: `currentMarkdown()`
 * returns the doc verbatim, and `change` emits it on every edit. Syntax markers
 * are hidden + styled off the active line and revealed (raw, editable) on it by
 * the `livePreview` extension. Designed to be generic so both Notes and Tasks can
 * reuse it; nothing here is notes-specific.
 */

const props = withDefaults(
  defineProps<{
    /** Markdown source. The editor doc is initialised from and synced to this. */
    body: string;
    placeholder?: string;
    editable?: boolean;
    /** Focus the editor on mount and on document switch (Obsidian-style). */
    autofocus?: boolean;
    /** Show the reading-width toggle. Off for hosts (e.g. tasks) whose column is
     * not a full document and must not stretch to the viewport. */
    widthToggle?: boolean;
    /** Live id → current-title map so id-bound wikilinks render the target's
     * current title instead of the snapshot baked into the markdown. */
    wikilinkTitles?: Record<string, string>;
    /** CSS min-height of the writing surface. Full-page hosts (Notes) want a tall
     * surface (60vh); embedded hosts (task description) pass a compact value so the
     * editor hugs its content instead of leaving a large empty area. */
    minHeight?: string;
    /** Render the mode/width controls inside the editor body. Hosts with their own
     * toolbar (Notes) set this false and drive `mode`/`reading` via v-model so the
     * controls live in the toolbar instead; embedded hosts (tasks) keep them here. */
    embeddedControls?: boolean;
    /** Optional image upload hook. When provided, pasting or dropping image files
     * uploads each via this callback and inserts `![name](url)` at the caret/drop
     * point. Returns the image URL, or null on failure (the host surfaces the
     * error). Hosts that omit it (task description) leave paste/drop untouched. */
    uploadImage?: (file: File) => Promise<string | null>;
  }>(),
  {
    placeholder: '',
    editable: true,
    autofocus: false,
    widthToggle: true,
    wikilinkTitles: () => ({}),
    minHeight: '60vh',
    embeddedControls: true,
  },
);

const emit = defineEmits<{
  /** Emitted on every doc change with the full markdown source. */
  change: [markdown: string];
  /** Emitted when a rendered wikilink is clicked, with its parsed reference. */
  'navigate-wikilink': [ref: WikilinkRef];
  /**
   * Emitted as the `[[` query changes; null clears the autocomplete. The second
   * argument is the caret's viewport position so the host can anchor the
   * suggestion dropdown next to the cursor (null when there is no trigger).
   */
  'wikilink-query': [query: string | null, caret: { left: number; top: number } | null];
}>();

const ui = useUiStore();

const host = ref<HTMLElement | null>(null);
let view: EditorView | null = null;
let activeTrigger: WikilinkTrigger | null = null;

/** Rendering mode: 'live' shows the live-preview decorations, 'source' the raw
 * markdown. A v-model so a host toolbar (Notes) can own it; defaults to local
 * state when unbound (tasks). */
const mode = defineModel<'live' | 'source'>('mode', { default: 'live' });
/** User-toggled read-only (reading/preview), layered on top of the host's
 * `editable` prop. v-model for the same reason as `mode`. */
const readonly = defineModel<boolean>('reading', { default: false });

// The placeholder string, quoted for use as a CSS `content` value (see <style>).
const placeholderCss = computed(() => JSON.stringify(props.placeholder));

// Compartments let us reconfigure the live-preview and edit-state extensions in
// place (mode / read-only toggles) without tearing down and rebuilding the view.
const livePreviewCompartment = new Compartment();
const editStateCompartment = new Compartment();

// The last markdown value this editor emitted, used to distinguish an external
// `body` prop change (must replace the doc) from an echo of our own edit (must
// be ignored, to avoid resetting the cursor).
let lastEmitted: string | null = null;

function currentMarkdown(): string {
  return view === null ? props.body : view.state.doc.toString();
}

/**
 * Reads the text before the cursor on the current line and emits the active `[[`
 * autocomplete query, reusing the same detection used by the ProseMirror editor.
 * Emits null when the selection is non-empty or no trigger is open.
 */
function syncWikilinkTrigger(state: EditorState): void {
  const range = state.selection.main;

  if (!range.empty) {
    activeTrigger = null;
    emit('wikilink-query', null, null);
    return;
  }

  const line = state.doc.lineAt(range.head);
  const textBefore = state.doc.sliceString(line.from, range.head);

  const trigger = detectWikilinkTrigger(textBefore, range.head);
  activeTrigger = trigger;

  if (trigger === null) {
    emit('wikilink-query', null, null);
    return;
  }

  // Anchor the suggestion dropdown just below the caret (viewport coords).
  const coords = view?.coordsAtPos(range.head) ?? null;
  const caret = coords === null ? null : { left: coords.left, top: coords.bottom + 4 };
  emit('wikilink-query', trigger.query, caret);
}

function onUpdate(docChanged: boolean, selectionChanged: boolean, state: EditorState): void {
  if (docChanged) {
    const md = state.doc.toString();
    lastEmitted = md;
    emit('change', md);
  }

  if (docChanged || selectionChanged) {
    syncWikilinkTrigger(state);
  }
}

function liveExtension(reveal: boolean) {
  return livePreview(
    { onWikilinkClick: (ref) => emit('navigate-wikilink', ref) },
    { reveal, titles: props.wikilinkTitles },
  );
}

/**
 * The rendering extension for the current mode:
 * - read-only → preview: live-preview decorations with NO active-line reveal, so
 *   the document reads as fully rendered (no markers, no caret-driven source).
 * - editable + live → live-preview with reveal-on-active-line for editing.
 * - editable + source → no decorations: raw markdown.
 */
function renderExtension() {
  if (readonly.value) return liveExtension(false);
  return mode.value === 'live' ? liveExtension(true) : [];
}

// Placeholder is rendered via CSS (`::after`, see <style>) rather than CodeMirror's
// widget placeholder: a widget at offset 0 of an otherwise-empty document sits on
// the cursor position and makes the caret unmeasurable, so the empty document would
// show no caret. This flags the content element as empty for the CSS to hook; the
// function is re-evaluated by CodeMirror on every update, so the class toggles as
// the document becomes empty / non-empty.
function emptyDocAttributes() {
  return EditorView.contentAttributes.of((v) =>
    v.state.doc.length === 0 ? { class: 'cm-doc-empty' } : null,
  );
}

/** Effective editability: the host must allow it AND read-only must be off. */
function effectiveEditable(): boolean {
  return props.editable && !readonly.value;
}

function editStateExtension(editable: boolean) {
  return [EditorView.editable.of(editable), EditorState.readOnly.of(!editable)];
}

function buildExtensions() {
  return [
    history(),
    keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
    markdown({ base: markdownLanguage, extensions: [GFM], codeLanguages: languages }),
    atlasHighlight,
    EditorView.lineWrapping,
    emptyDocAttributes(),
    livePreviewCompartment.of(renderExtension()),
    atlasMarkdownTheme,
    editStateCompartment.of(editStateExtension(effectiveEditable())),
    EditorView.domEventHandlers({
      paste: (event) => handleImageFiles(filesFromClipboard(event.clipboardData), null),
      drop: (event, v) => {
        if (props.uploadImage === undefined) return false;

        const files = filesFromDataTransfer(event.dataTransfer);
        if (files.filter(isImageFile).length === 0) return false;

        event.preventDefault();
        const pos = v.posAtCoords({ x: event.clientX, y: event.clientY });
        return handleImageFiles(files, pos);
      },
    }),
    EditorView.updateListener.of((update) => {
      onUpdate(update.docChanged, update.selectionSet, update.state);
    }),
  ];
}

function toggleMode(): void {
  mode.value = mode.value === 'live' ? 'source' : 'live';
}

function toggleReadonly(): void {
  readonly.value = !readonly.value;
}

// Reconfigure the live-preview / edit-state compartments whenever the mode or
// reading flags change — whether flipped by the in-body buttons or by a host
// toolbar through the v-models.
watch(mode, () => {
  view?.dispatch({ effects: livePreviewCompartment.reconfigure(renderExtension()) });
});

watch(readonly, () => {
  view?.dispatch({
    effects: [
      editStateCompartment.reconfigure(editStateExtension(effectiveEditable())),
      livePreviewCompartment.reconfigure(renderExtension()),
    ],
  });
});

/**
 * Replaces the open `[[query` trigger text with the chosen reference. An id-bound
 * ref serializes to `[[uuid|Title]]` (stable across renames); a title-only ref to
 * `[[Title]]`. No-op when no trigger is active.
 */
function insertWikilink(ref: WikilinkRef): void {
  if (view === null || activeTrigger === null) return;

  const from = activeTrigger.from;
  const to = view.state.selection.main.head;
  const insert = formatWikilink(ref);

  view.dispatch({
    changes: { from, to, insert },
    selection: { anchor: from + insert.length },
  });
  view.focus();
}

function focus(): void {
  view?.focus();
}

/**
 * Handles image files arriving by paste or drop when the host supplies an
 * `uploadImage` callback. Returns true when it takes over the event (images
 * present and a handler set), so CodeMirror's default paste/drop is suppressed;
 * false otherwise, leaving normal text paste/drop — and drops meant for an outer
 * dropzone (task attachments) — untouched.
 */
function handleImageFiles(files: File[], pos: number | null): boolean {
  if (props.uploadImage === undefined) return false;

  const images = files.filter(isImageFile);
  if (images.length === 0) return false;

  void uploadAndInsertImages(images, pos);
  return true;
}

async function uploadAndInsertImages(images: File[], pos: number | null): Promise<void> {
  const upload = props.uploadImage;
  if (upload === undefined || view === null) return;

  let at = pos ?? view.state.selection.main.head;

  for (const file of images) {
    const url = await upload(file);
    if (url === null || view === null) continue;

    const insert = imageSnippet(view.state, at, imageAlt(file), url);
    view.dispatch({
      changes: { from: at, to: at, insert },
      selection: { anchor: at + insert.length },
    });
    at += insert.length;
  }

  view?.focus();
}

/** A Markdown image on its own line — a leading newline is added only when the
 * insertion point is mid-line — so the live-preview block widget renders it. */
function imageSnippet(state: EditorState, at: number, alt: string, url: string): string {
  const atLineStart = at === state.doc.lineAt(at).from;
  return `${atLineStart ? '' : '\n'}![${alt}](${url})\n`;
}

/** Alt text from the file name (extension stripped), with characters that would
 * break the `![...]` syntax removed. */
function imageAlt(file: File): string {
  return file.name
    .replace(/\.[^.]+$/, '')
    .replaceAll(']', '')
    .replaceAll('\n', ' ')
    .replaceAll('\r', ' ')
    .trim();
}

defineExpose({ currentMarkdown, insertWikilink, focus });

onMounted(() => {
  if (host.value === null) return;

  view = new EditorView({
    state: EditorState.create({ doc: props.body, extensions: buildExtensions() }),
    parent: host.value,
  });
  lastEmitted = props.body;

  if (props.autofocus && effectiveEditable()) view.focus();
});

watch(
  () => props.body,
  (body) => {
    if (view === null) return;
    if (body === lastEmitted) return;
    if (body === view.state.doc.toString()) return;

    const nextSelection = restoreSelection(snapshotSelection(view.state.selection), body.length);
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: body },
      selection: nextSelection,
    });
    lastEmitted = body;

    if (props.autofocus && effectiveEditable()) view.focus();
  },
);

// Re-decorate when resolved wikilink titles arrive so id-bound links switch from
// their snapshot title to the target's current title without a reload.
watch(
  () => props.wikilinkTitles,
  () => {
    view?.dispatch({ effects: livePreviewCompartment.reconfigure(renderExtension()) });
  },
  { deep: true },
);

onBeforeUnmount(() => {
  view?.destroy();
  view = null;
});
</script>

<template>
  <div class="markdown-editor-wrap">
    <div v-if="embeddedControls" class="editor-controls">
      <button
        v-if="widthToggle"
        type="button"
        class="atl-gbtn"
        :class="{ on: ui.editorWide }"
        :title="ui.editorWide ? 'Readable width' : 'Wide width'"
        :aria-label="ui.editorWide ? 'Readable width' : 'Wide width'"
        @click="ui.toggleEditorWide()"
      >
        <Icon :name="ui.editorWide ? 'fold-horizontal' : 'unfold-horizontal'" :size="14" />
      </button>
      <button
        v-if="!readonly"
        type="button"
        class="atl-gbtn"
        :class="{ on: mode === 'source' }"
        :title="mode === 'live' ? 'Show markdown source' : 'Show preview'"
        :aria-label="mode === 'live' ? 'Show markdown source' : 'Show preview'"
        @click="toggleMode"
      >
        <Icon :name="mode === 'live' ? 'code' : 'eye'" :size="14" />
      </button>
      <button
        v-if="editable"
        type="button"
        class="atl-gbtn"
        :class="{ on: readonly }"
        :title="readonly ? 'Preview — click to edit' : 'Editing — click to preview'"
        :aria-label="readonly ? 'Preview — click to edit' : 'Editing — click to preview'"
        @click="toggleReadonly"
      >
        <Icon :name="readonly ? 'book-open' : 'pencil'" :size="14" />
      </button>
    </div>
    <div
      ref="host"
      class="markdown-editor"
      :class="{ 'is-preview': readonly }"
      :style="{ '--md-placeholder': placeholderCss, '--md-min-h': minHeight }"
    />
  </div>
</template>

<style scoped>
.editor-controls {
  display: flex;
  justify-content: flex-end;
  gap: 4px;
  margin-bottom: 6px;
}

.markdown-editor {
  min-height: var(--md-min-h, 60vh);
}

.markdown-editor :deep(.cm-editor) {
  min-height: var(--md-min-h, 60vh);
}

/* The writing surface is a document, not a form field: never show the global
   focus ring (base.css :focus-visible box-shadow) around any part of the editor,
   whether editable or in read-only preview. */
.markdown-editor :deep(.cm-editor),
.markdown-editor :deep(.cm-editor.cm-focused),
.markdown-editor :deep(.cm-scroller),
.markdown-editor :deep(.cm-content) {
  outline: none !important;
  box-shadow: none !important;
}

/* Preview (reading) mode: no caret — there is nothing to edit. */
.markdown-editor.is-preview :deep(.cm-content) {
  caret-color: transparent;
}

/* CSS placeholder for the empty document. Rendered as an overlay so it does not
   occupy a position in the content model (which would hide the caret at offset 0). */
.markdown-editor :deep(.cm-content.cm-doc-empty .cm-line:first-of-type) {
  position: relative;
}

.markdown-editor :deep(.cm-content.cm-doc-empty .cm-line:first-of-type)::after {
  content: var(--md-placeholder, '');
  position: absolute;
  left: 0;
  top: 0;
  color: var(--c-muted);
  pointer-events: none;
}
</style>
