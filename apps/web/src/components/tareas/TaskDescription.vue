<script setup lang="ts">
import { onBeforeUnmount, ref, toRef } from 'vue';
import { useRouter } from 'vue-router';
import type { ImageUploadResult } from '@/components/editor/imageUpload';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import MarkdownEditor from '@/components/editor/MarkdownEditor.vue';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import WikiLinkSuggest from '@/components/notas/WikiLinkSuggest.vue';
import { useApiImageSrc } from '@/composables/useApiImageSrc';
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
  /** Uploads an image pasted or dropped into the body as a task attachment. */
  uploadImage?: (file: File) => Promise<ImageUploadResult>;
}>();

const router = useRouter();
const tasks = useTasksStore();

// Attachment references point at the API, which the webview cannot load directly.
const resolveImageSrc = useApiImageSrc();

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
  const href = wikilinkHref(ref);
  if (href === null) return;

  void router.push(href);
}

/**
 * Places host-supplied markdown (an attachment reference) at the caret and saves
 * it on the same debounce as a keystroke, so the insertion persists on its own.
 */
function insertMarkdown(markdown: string): void {
  const editor = editorRef.value;
  if (editor === null) return;

  editor.insertAtCaret(markdown);
  onChange(editor.currentMarkdown());
}

defineExpose({ insertMarkdown });
</script>

<template>
  <div style="position: relative;" @keydown="onEditorKeydown">
    <MarkdownEditor
      ref="editorRef"
      :body="markdown"
      :wikilink-titles="wikilinkTitles"
      :editable="true"
      :width-toggle="false"
      :resolve-image-src="resolveImageSrc"
      :upload-image="uploadImage"
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
        :attachment-owner="{ kind: 'task', readableId }"
        @select="onSuggestSelect"
      />
    </div>
  </div>
</template>
