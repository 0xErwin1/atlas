<script setup lang="ts">
import { ref } from 'vue';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import MarkdownEditor from '@/components/editor/MarkdownEditor.vue';
import type { WikilinkRef } from '@/lib/wikilink';

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
  /** Live id → current-title map for id-bound wikilinks. */
  wikilinkTitles?: Record<string, string>;
  /** Uploads a pasted/dropped image and resolves to its URL (see MarkdownEditor). */
  uploadImage?: (file: File) => Promise<string | null>;
}>();

defineEmits<{
  /** Emitted on every edit with the current markdown body. */
  change: [markdown: string];
  /** Emitted when a rendered wikilink is clicked, with the parsed reference. */
  'navigate-wikilink': [ref: WikilinkRef];
  /** Emitted as the `[[` query changes; null clears the autocomplete. Carries
   * the caret viewport position so the host can anchor the dropdown. */
  'wikilink-query': [query: string | null, caret: { left: number; top: number } | null];
}>();

// View-mode models forwarded to the shared editor so the Notes toolbar owns the
// width/source/preview controls (the editor body renders none here).
const mode = defineModel<'live' | 'source'>('mode', { default: 'live' });
const reading = defineModel<boolean>('reading', { default: false });

const editorRef = ref<InstanceType<typeof MarkdownEditor> | null>(null);

function currentMarkdown(): string {
  return editorRef.value?.currentMarkdown() ?? props.body;
}

function insertWikilink(ref: WikilinkRef): void {
  editorRef.value?.insertWikilink(ref);
}

defineExpose({ currentMarkdown, insertWikilink });
</script>

<template>
  <MarkdownEditor
    ref="editorRef"
    v-model:mode="mode"
    v-model:reading="reading"
    :body="body"
    :wikilink-titles="props.wikilinkTitles"
    :upload-image="props.uploadImage"
    :embedded-controls="false"
    autofocus
    placeholder="Start writing…"
    @change="(md) => $emit('change', md)"
    @navigate-wikilink="(ref) => $emit('navigate-wikilink', ref)"
    @wikilink-query="(query, caret) => $emit('wikilink-query', query, caret)"
  />
</template>
