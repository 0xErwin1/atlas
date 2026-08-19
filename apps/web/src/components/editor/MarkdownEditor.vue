<script setup lang="ts">
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { syntaxTree } from '@codemirror/language';
import { languages } from '@codemirror/language-data';
import { Compartment, EditorState } from '@codemirror/state';
import { EditorView, keymap, lineNumbers } from '@codemirror/view';
import { GFM } from '@lezer/markdown';
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue';
import Icon from '@/components/ui/Icon.vue';
import { caretScrollDelta, createScrollableAncestorResolver } from '@/composables/caretScrolling';
import { restoreSelection, snapshotSelection } from '@/lib/editorSelection';
import { filesFromClipboard, filesFromDataTransfer, isImageFile } from '@/lib/fileTransfer';
import { blockInsertion } from '@/lib/markdownInsert';
import {
  detectWikilinkTrigger,
  formatWikilink,
  type WikilinkRef,
  type WikilinkTrigger,
} from '@/lib/wikilink';
import { useUiStore } from '@/stores/ui';
import { cspNonceExtension } from './cspNonce';
import { atlasHighlight } from './highlight';
import { type ImageUploadResult, imageUploadInsertion } from './imageUpload';
import { livePreview } from './livePreviewExtension';
import { atlasMarkdownTheme } from './theme';
import { unwrapParagraphs } from './unwrapParagraphs';

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
     * point. Legacy callers may return a URL string; structured results may also
     * provide canonical Markdown. Null leaves the document unchanged. Hosts that
     * omit it (task description) leave paste/drop untouched. */
    uploadImage?: (file: File) => Promise<ImageUploadResult>;
    /** Keep the caret in view by scrolling the host container while typing. On for
     * long-form surfaces (note body, task description); off for compact fixed
     * editors (the comment composer/edit box) that never outgrow their host. */
    followCaret?: boolean;
    /** Optional translation of a rendered image's source. Hosts whose images are
     * served by the Atlas API supply one so the bytes travel the platform
     * transport instead of the webview's own origin; see `useApiImageSrc`. */
    resolveImageSrc?: (url: string) => Promise<string | null>;
    /** Show the line-number gutter. On for hosts whose content is addressed by
     * line elsewhere (a note, which `read_document_lines` reads by line range);
     * off for hosts where a line number means nothing (a task description, a
     * comment). */
    lineNumbers?: boolean;
  }>(),
  {
    placeholder: '',
    editable: true,
    autofocus: false,
    widthToggle: true,
    wikilinkTitles: () => ({}),
    minHeight: '60vh',
    embeddedControls: true,
    followCaret: true,
    lineNumbers: false,
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
const gutterCompartment = new Compartment();

// The last markdown value this editor emitted, used to distinguish an external
// `body` prop change (must replace the doc) from an echo of our own edit (must
// be ignored, to avoid resetting the cursor).
let lastEmitted: string | null = null;
let imageUploadGeneration = 0;

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

  // The trigger scan is a line-local string read; the code-context check walks
  // the syntax tree. Running the cheap one first keeps the walk off the keystroke
  // path for every line that has no open `[[` on it.
  const trigger = detectWikilinkTrigger(textBefore, range.head);

  if (trigger === null || isCodeContext(state, range.head)) {
    activeTrigger = null;
    emit('wikilink-query', null, null);
    return;
  }

  activeTrigger = trigger;

  // Anchor the suggestion dropdown just below the caret (viewport coords).
  const coords = view?.coordsAtPos(range.head) ?? null;
  const caret = coords === null ? null : { left: coords.left, top: coords.bottom + 4 };
  emit('wikilink-query', trigger.query, caret);
}

function isCodeContext(state: EditorState, pos: number): boolean {
  let node: ReturnType<ReturnType<typeof syntaxTree>['resolve']> | null = syntaxTree(state).resolveInner(
    pos,
    -1,
  );
  while (node !== null) {
    if (node.name === 'InlineCode' || node.name === 'FencedCode' || node.name === 'CodeBlock') return true;
    node = node.parent;
  }
  return false;
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

// The editor grows with its content (no inner scroll), so the surface that moves
// is a host container — a note page, the task body — not CodeMirror's own
// scroller. Resolving it walks the ancestors with `getComputedStyle`, which the
// resolver keeps out of the keystroke path.
const resolveScroller = createScrollableAncestorResolver();

const CARET_MARGIN = 28;

/**
 * Keeps the caret visible while typing. CodeMirror's built-in scroll-into-view
 * acts on its own (non-scrolling) scroller and never moves the host, so a caret
 * at the bottom slips below the fold. This nudges the nearest scrollable ancestor
 * just enough to bring the caret back within a margin of its edges, and leaves
 * the scroll position untouched while the caret is already inside it.
 */
function keepCaretInView(): void {
  if (view === null || host.value === null) return;

  const caret = view.coordsAtPos(view.state.selection.main.head);
  if (caret === null) return;

  const scroller = resolveScroller(host.value);
  if (scroller === null) return;

  const delta = caretScrollDelta(caret, scroller.getBoundingClientRect(), CARET_MARGIN);
  if (delta === 0) return;

  scroller.scrollTop += delta;
}

// A keystroke arrives far more often than the screen repaints, and every run of
// `keepCaretInView` measures the caret and the scroller box. One pending frame at
// a time collapses a burst of characters into a single measurement.
let pendingCaretFrame: number | null = null;

function scheduleKeepCaretInView(): void {
  if (pendingCaretFrame !== null) return;

  pendingCaretFrame = requestAnimationFrame(() => {
    pendingCaretFrame = null;
    keepCaretInView();
  });
}

function liveExtension(reveal: boolean) {
  return livePreview(
    {
      onWikilinkClick: (ref) => emit('navigate-wikilink', ref),
      resolveImageSrc: props.resolveImageSrc,
    },
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

function gutterExtension() {
  return props.lineNumbers ? [lineNumbers()] : [];
}

function editStateExtension(editable: boolean) {
  return [EditorView.editable.of(editable), EditorState.readOnly.of(!editable)];
}

function buildExtensions() {
  return [
    cspNonceExtension(),
    history(),
    keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
    markdown({ base: markdownLanguage, extensions: [GFM], codeLanguages: languages }),
    atlasHighlight,
    EditorView.lineWrapping,
    emptyDocAttributes(),
    livePreviewCompartment.of(renderExtension()),
    gutterCompartment.of(gutterExtension()),
    atlasMarkdownTheme,
    editStateCompartment.of(editStateExtension(effectiveEditable())),
    EditorView.domEventHandlers({
      paste: (event) => takeOver(event, handleImageFiles(filesFromClipboard(event.clipboardData), null)),
      drop: (event, v) => {
        if (props.uploadImage === undefined) return false;

        const files = filesFromDataTransfer(event.dataTransfer);
        if (files.filter(isImageFile).length === 0) return false;

        event.preventDefault();
        const pos = v.posAtCoords({ x: event.clientX, y: event.clientY });
        return takeOver(event, handleImageFiles(files, pos));
      },
    }),
    EditorView.updateListener.of((update) => {
      onUpdate(update.docChanged, update.selectionSet, update.state);

      // Follow the caret only on the user's own edits — never on a programmatic
      // body replacement (external sync), which would yank the scroll position.
      if (
        props.followCaret &&
        update.docChanged &&
        update.transactions.some((tr) => tr.isUserEvent('input') || tr.isUserEvent('delete'))
      ) {
        scheduleKeepCaretInView();
      }
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

watch(
  () => props.lineNumbers,
  () => {
    view?.dispatch({ effects: gutterCompartment.reconfigure(gutterExtension()) });
  },
);

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
 * Inserts host-supplied markdown at the caret as its own block, breaking the
 * current line first so a reference never lands in the middle of a sentence.
 * Used by the task attachment list to reference a file from the description.
 */
function insertAtCaret(text: string): void {
  if (view === null || !effectiveEditable()) return;

  const at = view.state.selection.main.head;
  const insert = blockInsertion(text, at === view.state.doc.lineAt(at).from);

  view.dispatch({
    changes: { from: at, to: at, insert },
    selection: { anchor: at + insert.length },
  });
  view.focus();
}

/**
 * Joins every hard-wrapped paragraph into a single source line (see
 * `unwrapParagraphs`). Returns whether the document changed. No-op while the
 * editor is not editable, so reading mode cannot rewrite the note.
 */
function unwrapEditorParagraphs(): boolean {
  if (view === null || !effectiveEditable()) return false;
  return unwrapParagraphs(view);
}

/**
 * Stops a paste/drop the editor has taken over from also reaching an outer
 * dropzone. The task detail attaches files dropped or pasted anywhere on the
 * body, so without this an image dropped on the description would upload twice.
 */
function takeOver(event: Event, handled: boolean): boolean {
  if (handled) event.stopPropagation();
  return handled;
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

  void uploadAndInsertImages(images, pos, imageUploadGeneration);
  return true;
}

async function uploadAndInsertImages(images: File[], pos: number | null, generation: number): Promise<void> {
  const upload = props.uploadImage;
  if (upload === undefined || view === null) return;

  let at = pos ?? view.state.selection.main.head;

  for (const file of images) {
    let result: ImageUploadResult;
    try {
      result = await upload(file);
    } catch {
      continue;
    }

    if (
      result === null ||
      view === null ||
      generation !== imageUploadGeneration ||
      upload !== props.uploadImage ||
      !effectiveEditable()
    )
      continue;

    const insert = imageUploadInsertion(result, file.name, at === view.state.doc.lineAt(at).from);
    if (insert === null) continue;

    view.dispatch({
      changes: { from: at, to: at, insert },
      selection: { anchor: at + insert.length },
    });
    at += insert.length;
  }

  view?.focus();
}

defineExpose({
  currentMarkdown,
  insertWikilink,
  insertAtCaret,
  focus,
  unwrapParagraphs: unwrapEditorParagraphs,
});

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

    imageUploadGeneration += 1;
    const nextSelection = restoreSelection(snapshotSelection(view.state.selection), body.length);
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: body },
      selection: nextSelection,
    });
    lastEmitted = body;

    if (props.autofocus && effectiveEditable()) view.focus();
  },
);

watch(
  () => props.uploadImage,
  () => {
    imageUploadGeneration += 1;
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
  imageUploadGeneration += 1;

  if (pendingCaretFrame !== null) cancelAnimationFrame(pendingCaretFrame);
  pendingCaretFrame = null;

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
