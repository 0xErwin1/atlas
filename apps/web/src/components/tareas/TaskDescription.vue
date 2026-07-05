<script setup lang="ts">
import { onBeforeUnmount, ref, toRef } from 'vue';
import { useRouter } from 'vue-router';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import MarkdownEditor from '@/components/editor/MarkdownEditor.vue';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import WikiLinkSuggest from '@/components/notas/WikiLinkSuggest.vue';
import { useWikilinkSuggest } from '@/composables/useWikilinkSuggest';
import { useWikilinkTitles } from '@/composables/useWikilinkTitles';
import { type WikilinkRef, wikilinkHref } from '@/lib/wikilink';
import { useTasksStore } from '@/stores/tasks';

const props = defineProps<{
  /** Raw markdown description. */
  markdown: string;
  /** Workspace slug, required for the auto-save PATCH. */
  ws: string;
  /** Human-readable task ID, required for the auto-save PATCH. */
  readableId: string;
}>();

const router = useRouter();
const tasks = useTasksStore();

const wikilinkTitles = useWikilinkTitles(toRef(props, 'ws'), toRef(props, 'markdown'));

const editorRef = ref<InstanceType<typeof MarkdownEditor> | null>(null);
const suggestRef = ref<InstanceType<typeof WikiLinkSuggest> | null>(null);

// `[[wikilink]]` autocomplete, shared with the note editor.
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
let saveTimer: ReturnType<typeof setTimeout> | null = null;
let pendingSave: (() => void) | null = null;

function onChange(currentMarkdown: string): void {
  // Bind the target task now, not at fire time: this component can be reused for
  // a different task, and reading props later would save into the wrong one.
  const ws = props.ws;
  const readableId = props.readableId;
  pendingSave = () => void tasks.updateDescription(ws, readableId, currentMarkdown);

  if (saveTimer !== null) clearTimeout(saveTimer);
  saveTimer = setTimeout(flushSave, 800);
}

/**
 * Persist the pending edit immediately, cancelling the debounce. Called on the
 * trailing debounce and on unmount so closing or switching a task within the
 * debounce window never drops the last keystrokes.
 */
function flushSave(): void {
  if (saveTimer !== null) {
    clearTimeout(saveTimer);
    saveTimer = null;
  }

  const save = pendingSave;
  pendingSave = null;
  save?.();
}

onBeforeUnmount(flushSave);

function onNavigateWikilink(ref: WikilinkRef): void {
  void router.push(wikilinkHref(ref));
}
</script>

<template>
  <div style="position: relative;" @keydown="onEditorKeydown">
    <MarkdownEditor
      ref="editorRef"
      :body="markdown"
      :wikilink-titles="wikilinkTitles"
      :editable="true"
      :width-toggle="false"
      min-height="2.5rem"
      placeholder="Add a description…"
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
