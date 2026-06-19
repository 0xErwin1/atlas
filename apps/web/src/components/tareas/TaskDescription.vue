<script setup lang="ts">
import { ref, toRef } from 'vue';
import { useRouter } from 'vue-router';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import MarkdownEditor from '@/components/editor/MarkdownEditor.vue';
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
let saveTimer: ReturnType<typeof setTimeout> | null = null;

function onChange(currentMarkdown: string): void {
  if (saveTimer !== null) clearTimeout(saveTimer);
  saveTimer = setTimeout(
    () => void tasks.updateDescription(props.ws, props.readableId, currentMarkdown),
    800,
  );
}

function onNavigateWikilink(ref: WikilinkRef): void {
  void router.push(wikilinkHref(ref));
}
</script>

<template>
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
  />
</template>
