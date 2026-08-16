<script setup lang="ts">
import { onBeforeUnmount, ref } from 'vue';
import type { ImageUploadResult } from '@/components/editor/imageUpload';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import WikilinkEditor from '@/components/editor/WikilinkEditor.vue';
import { useApiImageSrc } from '@/composables/useApiImageSrc';
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

const tasks = useTasksStore();

// Attachment references point at the API, which the webview cannot load directly.
const resolveImageSrc = useApiImageSrc();

const editorRef = ref<InstanceType<typeof WikilinkEditor> | null>(null);
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
  <WikilinkEditor
    ref="editorRef"
    :ws="ws"
    :body="markdown"
    :attachment-owner="{ kind: 'task', readableId }"
    :editable="true"
    :width-toggle="false"
    :resolve-image-src="resolveImageSrc"
    :upload-image="uploadImage"
    min-height="2.5rem"
    placeholder="Add a description…"
    @change="onChange"
  />
</template>
