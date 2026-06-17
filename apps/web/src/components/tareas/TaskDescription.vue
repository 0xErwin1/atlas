<script setup lang="ts">
import { ref } from 'vue';
import { useRouter } from 'vue-router';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import MarkdownEditor from '@/components/editor/MarkdownEditor.vue';
import { wikilinkTarget } from '@/lib/wikilink';
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

const editorRef = ref<InstanceType<typeof MarkdownEditor> | null>(null);
let saveTimer: ReturnType<typeof setTimeout> | null = null;

function onChange(currentMarkdown: string): void {
  if (saveTimer !== null) clearTimeout(saveTimer);
  saveTimer = setTimeout(
    () => void tasks.updateDescription(props.ws, props.readableId, currentMarkdown),
    800,
  );
}

function onNavigateWikilink(title: string): void {
  void router.push(wikilinkTarget(title));
}
</script>

<template>
  <MarkdownEditor
    ref="editorRef"
    :body="markdown"
    :editable="true"
    placeholder="Add a description…"
    @change="onChange"
    @navigate-wikilink="onNavigateWikilink"
  />
</template>
