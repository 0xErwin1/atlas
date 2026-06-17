<script setup lang="ts">
import { ref } from 'vue';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import MarkdownEditor from '@/components/editor/MarkdownEditor.vue';

/**
 * Notes editor: a thin wrapper around the shared CodeMirror 6 `MarkdownEditor`.
 *
 * It exists so `Notes.vue` keeps a stable, notes-shaped API (`currentMarkdown`,
 * `insertWikilink`, and the three editor emits) while the actual editing surface
 * is the generic markdown editor shared with Tasks. The markdown source is the
 * source of truth — `currentMarkdown()` returns exactly the editor's doc text,
 * which the CAS save path in `Notes.vue` persists.
 */

const props = defineProps<{
  /** Markdown body (frontmatter already stripped by useMarkdownDoc). */
  body: string;
}>();

defineEmits<{
  /** Emitted on every edit with the current markdown body. */
  change: [markdown: string];
  /** Emitted when a rendered wikilink is clicked, with the target title. */
  'navigate-wikilink': [title: string];
  /** Emitted as the `[[` query changes; null clears the autocomplete. */
  'wikilink-query': [query: string | null];
}>();

const editorRef = ref<InstanceType<typeof MarkdownEditor> | null>(null);

function currentMarkdown(): string {
  return editorRef.value?.currentMarkdown() ?? props.body;
}

function insertWikilink(title: string): void {
  editorRef.value?.insertWikilink(title);
}

defineExpose({ currentMarkdown, insertWikilink });
</script>

<template>
  <MarkdownEditor
    ref="editorRef"
    :body="body"
    autofocus
    placeholder="Start writing…"
    @change="(md) => $emit('change', md)"
    @navigate-wikilink="(title) => $emit('navigate-wikilink', title)"
    @wikilink-query="(query) => $emit('wikilink-query', query)"
  />
</template>
