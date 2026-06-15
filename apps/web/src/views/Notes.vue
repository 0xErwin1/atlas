<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import BacklinksPanel from '@/components/notas/BacklinksPanel.vue';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import NoteEditor from '@/components/notas/NoteEditor.vue';
import PropertiesPanel from '@/components/notas/PropertiesPanel.vue';
// biome-ignore lint/style/useImportType: used as a component in <template>, not only as a type
import WikiLinkSuggest from '@/components/notas/WikiLinkSuggest.vue';
import EditorToolbar from '@/components/shell/EditorToolbar.vue';
import Btn from '@/components/ui/Btn.vue';
import Icon from '@/components/ui/Icon.vue';
import { useMarkdownDoc } from '@/composables/useMarkdownDoc';
import { wikilinkTarget } from '@/lib/wikilink';
import { useDocumentsStore } from '@/stores/documents';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';
import AppShell from '@/views/AppShell.vue';
import NotesSidebar from '@/views/NotesSidebar.vue';

const route = useRoute();
const router = useRouter();
const workspace = useWorkspaceStore();
const documents = useDocumentsStore();
const ui = useUiStore();
const { load, save } = useMarkdownDoc();

const editorRef = ref<InstanceType<typeof NoteEditor> | null>(null);
const suggestRef = ref<InstanceType<typeof WikiLinkSuggest> | null>(null);

const slug = computed(() => {
  const s = route.params.slug;
  return typeof s === 'string' && s.length > 0 ? s : null;
});

const ws = computed(() => workspace.activeWorkspaceSlug ?? '');

const title = ref('');
const body = ref('');
const meta = ref<Record<string, unknown>>({});
const headRevisionId = ref('');
const dirty = ref(false);
const loadError = ref<string | null>(null);
const conflict = ref<{ hint: string } | null>(null);
const wikilinkQuery = ref<string | null>(null);

const breadcrumbs = computed(() => ['Atlas', title.value || 'Untitled']);

let saveTimer: ReturnType<typeof setTimeout> | null = null;

async function loadDoc(): Promise<void> {
  loadError.value = null;
  conflict.value = null;

  if (slug.value === null || ws.value === '') {
    body.value = '';
    title.value = '';
    meta.value = {};
    return;
  }

  try {
    const result = await load(ws.value, slug.value);
    body.value = result.body;
    meta.value = result.meta;
    headRevisionId.value = result.headRevisionId;
    title.value = typeof result.meta.title === 'string' ? result.meta.title : (slug.value ?? '');
    dirty.value = false;
    await documents.loadBacklinks(ws.value, slug.value);
  } catch (e) {
    loadError.value = e instanceof Error ? e.message : 'Failed to load document';
  }
}

async function persist(): Promise<void> {
  if (slug.value === null || ws.value === '') return;

  const current = editorRef.value?.currentMarkdown() ?? body.value;
  const result = await save(ws.value, slug.value, current, meta.value, headRevisionId.value);

  if (result.kind === 'ok') {
    dirty.value = false;
    conflict.value = null;
    return;
  }

  if (result.kind === 'conflict') {
    // The 3-way merge UI is T21-T22; here we surface a non-destructive banner
    // and never apply last-write-wins.
    conflict.value = { hint: result.problem.hint ?? 'This document changed on the server. Reload to merge.' };
    ui.showBanner(conflict.value.hint, 'warning');
    return;
  }

  ui.showBanner(result.hint ?? result.title, 'error');
}

function onChange(markdown: string): void {
  body.value = markdown;
  dirty.value = true;

  if (saveTimer !== null) clearTimeout(saveTimer);
  saveTimer = setTimeout(() => void persist(), 800);
}

function onNavigateWikilink(linkTitle: string): void {
  void router.push(wikilinkTarget(linkTitle));
}

function onWikilinkQuery(query: string | null): void {
  wikilinkQuery.value = query;
}

function onSuggestSelect(selectedTitle: string): void {
  editorRef.value?.insertWikilink(selectedTitle);
  wikilinkQuery.value = null;
}

function onEditorKeydown(event: KeyboardEvent): void {
  if (suggestRef.value?.open !== true) return;

  if (event.key === 'ArrowDown') {
    event.preventDefault();
    suggestRef.value.moveDown();
  } else if (event.key === 'ArrowUp') {
    event.preventDefault();
    suggestRef.value.moveUp();
  } else if (event.key === 'Enter') {
    event.preventDefault();
    suggestRef.value.confirmActive();
  } else if (event.key === 'Escape') {
    wikilinkQuery.value = null;
  }
}

watch([slug, ws], loadDoc, { immediate: true });
</script>

<template>
  <AppShell>
    <template #sidebar>
      <NotesSidebar />
    </template>

    <EditorToolbar :breadcrumbs="breadcrumbs" :dirty="dirty">
      <button
        type="button"
        title="Toggle inspector"
        aria-label="Toggle inspector"
        class="flex items-center justify-center"
        style="width: 28px; height: 28px; border: none; background: transparent; cursor: pointer; color: var(--c-muted);"
        @click="ui.toggleInspector()"
      >
        <Icon name="panel-right" :size="16" />
      </button>
    </EditorToolbar>

    <div class="flex-1 overflow-y-auto">
      <div style="max-width: 720px; margin: 0 auto; padding: 30px 40px; position: relative;">
        <p
          v-if="loadError"
          style="
            padding: 8px 12px;
            border-radius: var(--r-md);
            background: var(--c-banner-err-bg);
            color: var(--c-banner-err-fg);
            font-size: var(--fs-sm);
          "
        >
          {{ loadError }}
        </p>

        <div
          v-if="conflict"
          class="flex items-center justify-between gap-2"
          style="
            padding: 8px 12px;
            margin-bottom: 12px;
            border: 1px solid var(--c-warning);
            border-radius: var(--r-md);
            background: var(--c-raised);
            color: var(--c-foreground);
            font-size: var(--fs-sm);
          "
        >
          <span>{{ conflict.hint }}</span>
          <Btn variant="secondary" @click="loadDoc">Reload</Btn>
        </div>

        <template v-if="slug">
          <h1
            style="font-size: var(--fs-title); font-weight: var(--fw-bold); color: var(--c-foreground); margin-bottom: 16px;"
          >
            {{ title || 'Untitled' }}
          </h1>

          <div @keydown="onEditorKeydown">
            <NoteEditor
              ref="editorRef"
              :body="body"
              @change="onChange"
              @navigate-wikilink="onNavigateWikilink"
              @wikilink-query="onWikilinkQuery"
            />

            <div style="position: absolute; left: 40px;">
              <WikiLinkSuggest
                ref="suggestRef"
                :ws="ws"
                :query="wikilinkQuery"
                @select="onSuggestSelect"
              />
            </div>
          </div>
        </template>

        <p
          v-else
          style="font-size: var(--fs-sm); color: var(--c-muted);"
        >
          Select a document from the tree to start editing.
        </p>
      </div>
    </div>

    <template #inspector-properties>
      <PropertiesPanel :meta="meta" />
    </template>

    <template #inspector-backlinks>
      <BacklinksPanel
        :backlinks="documents.backlinks"
        @navigate="(s) => router.push({ name: 'notes', params: { slug: s } })"
      />
    </template>
  </AppShell>
</template>
