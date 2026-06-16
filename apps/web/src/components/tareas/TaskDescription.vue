<script setup lang="ts">
import { EditorState } from '@tiptap/pm/state';
import { EditorView } from '@tiptap/pm/view';
import { onBeforeUnmount, onMounted, ref, watch } from 'vue';
import { atlasSchema, markdownToDoc } from '@/lib/markdownSerializer';

const props = defineProps<{
  /** Raw markdown description (frontmatter already stripped upstream). */
  markdown: string;
}>();

const host = ref<HTMLElement | null>(null);
let view: EditorView | null = null;

/**
 * Read-only ProseMirror view over the shared atlasSchema so the task
 * description renders markdown (headings, lists, code, wikilinks) without
 * introducing a second markdown engine. `editable: false` makes it display-only.
 */
function build(markdown: string): EditorState {
  return EditorState.create({
    doc: markdownToDoc(markdown),
    schema: atlasSchema,
  });
}

onMounted(() => {
  if (host.value === null) {
    return;
  }
  view = new EditorView(host.value, {
    state: build(props.markdown),
    editable: () => false,
  });
});

watch(
  () => props.markdown,
  (markdown) => {
    if (view === null) {
      return;
    }
    view.updateState(build(markdown));
  },
);

onBeforeUnmount(() => {
  view?.destroy();
  view = null;
});
</script>

<template>
  <div
    v-if="markdown"
    ref="host"
    class="task-description"
  />
  <p
    v-else
    style="font-size: var(--fs-sm); color: var(--c-muted); font-style: italic;"
  >
    No description.
  </p>
</template>

<style scoped>
.task-description :deep(.ProseMirror) {
  outline: none;
  color: var(--c-foreground);
  font-family: var(--font-mono);
  font-size: var(--fs-lg);
  line-height: var(--lh-relaxed);
}

.task-description :deep(.ProseMirror p) {
  margin: 0 0 12px;
}

.task-description :deep(.ProseMirror h1),
.task-description :deep(.ProseMirror h2),
.task-description :deep(.ProseMirror h3) {
  font-weight: var(--fw-bold);
  color: var(--c-foreground);
  margin: 18px 0 8px;
}

.task-description :deep(.ProseMirror code) {
  font-family: var(--font-mono);
  background: var(--c-input);
  border-radius: var(--r-sm);
  padding: 1px 4px;
}

.task-description :deep(.ProseMirror pre) {
  background: var(--c-raised);
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  padding: 10px 12px;
  overflow-x: auto;
}

.task-description :deep(.wikilink) {
  color: var(--c-info);
}
</style>
