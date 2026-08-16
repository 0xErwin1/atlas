<script setup lang="ts">
import { ref } from 'vue';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import WikilinkEditor from '@/components/editor/WikilinkEditor.vue';
import { useApiImageSrc } from '@/composables/useApiImageSrc';

/**
 * Notes editor: a thin wrapper around the shared `WikilinkEditor`.
 *
 * It exists so `Notes.vue` keeps a stable, notes-shaped API (`currentMarkdown`,
 * `unwrapParagraphs`, and the view-mode models) while the editing surface, the
 * `[[` picker and wikilink navigation are the ones shared with Tasks and
 * comments. The markdown source is the source of truth — `currentMarkdown()`
 * returns exactly the editor's doc text, which the CAS save path in `Notes.vue`
 * persists.
 */

const props = defineProps<{
  /** Workspace slug, for resolving and offering wikilinks. */
  ws: string;
  /** Markdown body (frontmatter already stripped by useMarkdownDoc). */
  body: string;
  /** Document slug, which scopes `[[file:` suggestions to its attachments. */
  slug: string;
  /** Uploads a pasted/dropped image and resolves to its URL (see MarkdownEditor). */
  uploadImage?: (file: File) => Promise<string | null>;
  /** Show the line-number gutter (a user preference owned by the Notes toolbar). */
  lineNumbers?: boolean;
}>();

defineEmits<{
  /** Emitted on every edit with the current markdown body. */
  change: [markdown: string];
}>();

// View-mode models forwarded to the shared editor so the Notes toolbar owns the
// width/source/preview controls (the editor body renders none here).
const mode = defineModel<'live' | 'source'>('mode', { default: 'live' });
const reading = defineModel<boolean>('reading', { default: false });

// Inline images are stored as `/api/…` attachments, which the webview cannot load
// directly.
const resolveImageSrc = useApiImageSrc();

const editorRef = ref<InstanceType<typeof WikilinkEditor> | null>(null);

function currentMarkdown(): string {
  return editorRef.value?.currentMarkdown() ?? props.body;
}

/** Joins every hard-wrapped paragraph into one source line; see `unwrapParagraphs`. */
function unwrapParagraphs(): boolean {
  return editorRef.value?.unwrapParagraphs() ?? false;
}

defineExpose({ currentMarkdown, unwrapParagraphs });
</script>

<template>
  <WikilinkEditor
    ref="editorRef"
    v-model:mode="mode"
    v-model:reading="reading"
    :ws="ws"
    :body="body"
    :attachment-owner="{ kind: 'document', slug }"
    :upload-image="props.uploadImage"
    :resolve-image-src="resolveImageSrc"
    :line-numbers="props.lineNumbers ?? false"
    :embedded-controls="false"
    autofocus
    placeholder="Start writing…"
    @change="(md: string) => $emit('change', md)"
  />
</template>
