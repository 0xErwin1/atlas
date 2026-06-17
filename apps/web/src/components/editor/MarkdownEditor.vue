<script setup lang="ts">
import { defaultKeymap, history, historyKeymap, indentWithTab } from '@codemirror/commands';
import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { EditorState } from '@codemirror/state';
import { EditorView, keymap, placeholder as placeholderExt } from '@codemirror/view';
import { GFM } from '@lezer/markdown';
import { onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { detectWikilinkTrigger, type WikilinkTrigger } from '@/lib/wikilink';
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
  }>(),
  { placeholder: '', editable: true },
);

const emit = defineEmits<{
  /** Emitted on every doc change with the full markdown source. */
  change: [markdown: string];
  /** Emitted when a rendered wikilink is clicked, with its title. */
  'navigate-wikilink': [title: string];
  /** Emitted as the `[[` query changes; null clears the autocomplete. */
  'wikilink-query': [query: string | null];
}>();

const host = ref<HTMLElement | null>(null);
let view: EditorView | null = null;
let activeTrigger: WikilinkTrigger | null = null;

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
    emit('wikilink-query', null);
    return;
  }

  const line = state.doc.lineAt(range.head);
  const textBefore = state.doc.sliceString(line.from, range.head);

  const trigger = detectWikilinkTrigger(textBefore, range.head);
  activeTrigger = trigger;
  emit('wikilink-query', trigger?.query ?? null);
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

function buildExtensions() {
  return [
    history(),
    keymap.of([...defaultKeymap, ...historyKeymap, indentWithTab]),
    markdown({ base: markdownLanguage, extensions: [GFM] }),
    EditorView.lineWrapping,
    placeholderExt(props.placeholder),
    livePreview({ onWikilinkClick: (title) => emit('navigate-wikilink', title) }),
    atlasMarkdownTheme,
    EditorView.editable.of(props.editable),
    EditorView.updateListener.of((update) => {
      onUpdate(update.docChanged, update.selectionSet, update.state);
    }),
  ];
}

/**
 * Replaces the open `[[query` trigger text with `[[Title]]`, then places the
 * cursor after the inserted text. No-op when no trigger is active.
 */
function insertWikilink(title: string): void {
  if (view === null || activeTrigger === null) return;

  const from = activeTrigger.from;
  const to = view.state.selection.main.head;
  const insert = `[[${title}]]`;

  view.dispatch({
    changes: { from, to, insert },
    selection: { anchor: from + insert.length },
  });
  view.focus();
}

function focus(): void {
  view?.focus();
}

defineExpose({ currentMarkdown, insertWikilink, focus });

onMounted(() => {
  if (host.value === null) return;

  view = new EditorView({
    state: EditorState.create({ doc: props.body, extensions: buildExtensions() }),
    parent: host.value,
  });
  lastEmitted = props.body;
});

watch(
  () => props.body,
  (body) => {
    if (view === null) return;
    if (body === lastEmitted) return;
    if (body === view.state.doc.toString()) return;

    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: body },
    });
    lastEmitted = body;
  },
);

onBeforeUnmount(() => {
  view?.destroy();
  view = null;
});
</script>

<template>
  <div
    ref="host"
    class="markdown-editor"
  />
</template>

<style scoped>
.markdown-editor {
  min-height: 60vh;
}

.markdown-editor :deep(.cm-editor) {
  min-height: 60vh;
}

/* The writing surface is a document, not a form field: never show the global
   focus ring (base.css :focus-visible box-shadow) around the editable area. */
.markdown-editor :deep(.cm-editor.cm-focused) {
  outline: none;
  box-shadow: none;
}

.markdown-editor :deep(.cm-content) {
  outline: none;
}
</style>
