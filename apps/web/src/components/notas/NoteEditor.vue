<script setup lang="ts">
import { baseKeymap, toggleMark } from '@tiptap/pm/commands';
import { history, redo, undo } from '@tiptap/pm/history';
import { keymap } from '@tiptap/pm/keymap';
import type { Node as PmNode } from '@tiptap/pm/model';
import { EditorState, type Transaction } from '@tiptap/pm/state';
import { EditorView } from '@tiptap/pm/view';
import { onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { atlasSchema, docToMarkdown, markdownToDoc } from '@/lib/markdownSerializer';
import { detectWikilinkTrigger, type WikilinkTrigger } from '@/lib/wikilink';

/**
 * Resolves a node/mark type from the atlas schema, failing loudly if it is
 * missing. These types are statically defined in `atlasSchema`, so a miss is a
 * programmer error, not a runtime condition.
 */
function requireMark(name: string) {
  const mark = atlasSchema.marks[name];
  if (mark === undefined) throw new Error(`atlasSchema is missing mark "${name}"`);
  return mark;
}

function requireNode(name: string) {
  const node = atlasSchema.nodes[name];
  if (node === undefined) throw new Error(`atlasSchema is missing node "${name}"`);
  return node;
}

const props = defineProps<{
  /** Markdown body (frontmatter already stripped by useMarkdownDoc). */
  body: string;
}>();

const emit = defineEmits<{
  /** Emitted on every edit with the current serialized markdown body. */
  change: [markdown: string];
  /** Emitted when a rendered wikilink is clicked, with the target title. */
  'navigate-wikilink': [title: string];
  /** Emitted as the `[[` query changes; null clears the autocomplete. */
  'wikilink-query': [query: string | null];
}>();

const host = ref<HTMLElement | null>(null);
let view: EditorView | null = null;
let activeTrigger: WikilinkTrigger | null = null;

/**
 * Builds the editor state from a markdown body, binding the EditorView to the
 * same `atlasSchema` the serializer uses so the md->doc->md round-trip stays
 * byte-stable (REQ-W15).
 */
function buildState(body: string): EditorState {
  const doc: PmNode = markdownToDoc(body);

  return EditorState.create({
    doc,
    schema: atlasSchema,
    plugins: [
      history(),
      keymap({
        'Mod-z': undo,
        'Mod-y': redo,
        'Mod-Shift-z': redo,
        'Mod-b': toggleMark(requireMark('strong')),
        'Mod-i': toggleMark(requireMark('em')),
      }),
      keymap(baseKeymap),
    ],
  });
}

function currentMarkdown(): string {
  if (view === null) return props.body;
  return docToMarkdown(view.state.doc);
}

/**
 * Reads the text immediately before the cursor on the current text block and
 * emits the active `[[` autocomplete query (REQ-W16), or null when no trigger
 * is open.
 */
function syncWikilinkTrigger(state: EditorState): void {
  const { from, empty } = state.selection;

  if (!empty) {
    activeTrigger = null;
    emit('wikilink-query', null);
    return;
  }

  const $from = state.selection.$from;
  const blockStart = $from.start();
  const textBefore = state.doc.textBetween(blockStart, from, '\n', '\n');

  const trigger = detectWikilinkTrigger(textBefore, from);
  activeTrigger = trigger;
  emit('wikilink-query', trigger?.query ?? null);
}

function onTransaction(tr: Transaction): void {
  if (view === null) return;

  const next = view.state.apply(tr);
  view.updateState(next);

  if (tr.docChanged) {
    emit('change', docToMarkdown(next.doc));
  }

  syncWikilinkTrigger(next);
}

/**
 * Inserts a wikilink node for the chosen title, replacing the open `[[query`
 * trigger text. No-op when no trigger is active.
 */
function insertWikilink(title: string): void {
  if (view === null || activeTrigger === null) return;

  const { from } = view.state.selection;
  const node = requireNode('wikilink').create({ title });

  const tr = view.state.tr.replaceWith(activeTrigger.from, from, node);
  view.dispatch(tr);
  view.focus();
}

defineExpose({ currentMarkdown, insertWikilink });

onMounted(() => {
  if (host.value === null) return;

  view = new EditorView(host.value, {
    state: buildState(props.body),
    dispatchTransaction: onTransaction,
    handleClickOn(_view, _pos, node) {
      if (node.type.name === 'wikilink') {
        emit('navigate-wikilink', node.attrs.title as string);
        return true;
      }
      return false;
    },
  });
});

watch(
  () => props.body,
  (body) => {
    if (view === null) return;
    if (docToMarkdown(view.state.doc) === body) return;
    view.updateState(buildState(body));
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
    class="note-editor"
  />
</template>

<style scoped>
.note-editor :deep(.ProseMirror) {
  outline: none;
  min-height: 60vh;
  color: var(--c-foreground);
  font-family: var(--font-mono);
  font-size: var(--fs-lg);
  line-height: var(--lh-relaxed);
}

/* The writing surface is a document, not a form field: never show the global
   focus ring (base.css :focus-visible box-shadow) around the editable area. */
.note-editor :deep(.ProseMirror:focus),
.note-editor :deep(.ProseMirror:focus-visible) {
  outline: none;
  box-shadow: none;
}

.note-editor :deep(.ProseMirror p) {
  margin: 0 0 12px;
}

.note-editor :deep(.ProseMirror h1),
.note-editor :deep(.ProseMirror h2),
.note-editor :deep(.ProseMirror h3) {
  font-weight: var(--fw-bold);
  color: var(--c-foreground);
  margin: 18px 0 8px;
}

.note-editor :deep(.ProseMirror code) {
  font-family: var(--font-mono);
  background: var(--c-input);
  border-radius: var(--r-sm);
  padding: 1px 4px;
}

.note-editor :deep(.ProseMirror pre) {
  background: var(--c-raised);
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  padding: 10px 12px;
  overflow-x: auto;
}

.note-editor :deep(.wikilink) {
  color: var(--c-info);
  cursor: pointer;
}

.note-editor :deep(.wikilink:hover) {
  text-decoration: underline;
}
</style>
