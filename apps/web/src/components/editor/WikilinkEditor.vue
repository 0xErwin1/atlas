<script setup lang="ts">
/**
 * A `MarkdownEditor` with wikilinks wired up: the `[[` picker, live title
 * resolution, and click-to-navigate.
 *
 * Every editor that wants wikilinks needs the same four pieces — the titles
 * composable, the suggest composable, the caret-anchored dropdown, and the
 * navigation handler — so they live here once instead of being re-inlined per
 * host. Anything not listed in the props below falls through to the underlying
 * `MarkdownEditor`.
 */
import { computed, ref } from 'vue';
import { useRouter } from 'vue-router';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import MarkdownEditor from '@/components/editor/MarkdownEditor.vue';
import type { AttachmentOwner } from '@/components/notas/WikiLinkSuggest.vue';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import WikiLinkSuggest from '@/components/notas/WikiLinkSuggest.vue';
import { useWikilinkSuggest } from '@/composables/useWikilinkSuggest';
import { useWikilinkTitles } from '@/composables/useWikilinkTitles';
import { wikilinkHref } from '@/lib/wikilink';

defineOptions({ inheritAttrs: false });

const props = defineProps<{
  ws: string;
  /** Initial markdown. The editor owns the text after mount. */
  body: string;
  /** Scopes `[[file:` suggestions; omit where the host has no attachments. */
  attachmentOwner?: AttachmentOwner | null;
}>();

const emit = defineEmits<{ change: [markdown: string] }>();

const router = useRouter();

// Titles resolve against what is on screen, not the last saved body, so a link
// typed a moment ago renders its target's title without waiting for a save.
const currentBody = ref(props.body);
const wikilinkTitles = useWikilinkTitles(
  computed(() => props.ws),
  currentBody,
);

const editorRef = ref<InstanceType<typeof MarkdownEditor> | null>(null);
const suggestRef = ref<InstanceType<typeof WikiLinkSuggest> | null>(null);

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

function onChange(markdown: string): void {
  currentBody.value = markdown;
  emit('change', markdown);
}

function onNavigateWikilink(reference: Parameters<typeof wikilinkHref>[0]): void {
  const href = wikilinkHref(reference);
  if (href === null) return;

  void router.push(href);
}

function focus(): void {
  editorRef.value?.focus();
}

function insertAtCaret(markdown: string): void {
  editorRef.value?.insertAtCaret(markdown);
}

function currentMarkdown(): string {
  return editorRef.value?.currentMarkdown() ?? currentBody.value;
}

/** Joins every hard-wrapped paragraph into one source line. */
function unwrapParagraphs(): boolean {
  return editorRef.value?.unwrapParagraphs() ?? false;
}

defineExpose({ focus, insertAtCaret, currentMarkdown, unwrapParagraphs });
</script>

<template>
  <div style="position: relative;" @keydown="onEditorKeydown">
    <MarkdownEditor
      ref="editorRef"
      v-bind="$attrs"
      :body="body"
      :wikilink-titles="wikilinkTitles"
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
        :attachment-owner="attachmentOwner ?? null"
        @select="onSuggestSelect"
      />
    </div>
  </div>
</template>
