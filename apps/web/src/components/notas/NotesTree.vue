<script setup lang="ts">
import { computed, nextTick, ref } from 'vue';
import ContextMenu, { type MenuItem } from '@/components/ui/ContextMenu.vue';
import Icon from '@/components/ui/Icon.vue';
import Row from '@/components/ui/Row.vue';
import SectionLabel from '@/components/ui/SectionLabel.vue';
import { buildNotesTree, type DocInput, type FolderInput } from '@/lib/notesTree';
import NoteTreeRow from './NoteTreeRow.vue';

const props = defineProps<{
  projectName: string;
  folders: FolderInput[];
  docs: DocInput[];
  activeSlug: string | null;
}>();

const emit = defineEmits<{
  'select-doc': [slug: string];
  'create-doc': [title: string, folderId?: string];
  'rename-doc': [slug: string, title: string];
  'remove-doc': [slug: string];
  'create-folder': [name: string, parentFolderId?: string];
  'rename-folder': [folderId: string, name: string];
  'remove-folder': [folderId: string];
}>();

const tree = computed(() => buildNotesTree(props.folders, props.docs));

const isEmpty = computed(() => tree.value.folders.length === 0 && tree.value.docs.length === 0);

const contextOpen = ref(false);
const contextX = ref(0);
const contextY = ref(0);

type ContextState = { kind: 'root' } | { kind: 'doc'; slug: string; title: string };
const contextState = ref<ContextState>({ kind: 'root' });

type InlineTarget = 'new-doc' | 'new-folder';
const inlineTarget = ref<InlineTarget | null>(null);
const inlineValue = ref('');
const inlineInputRef = ref<HTMLInputElement | null>(null);
const pendingDocRename = ref<{ slug: string } | null>(null);

const rootMenuItems = computed<MenuItem[]>(() => [
  { label: 'New page', icon: 'file-plus', action: () => openInline('new-doc') },
  { label: 'New folder', icon: 'folder-plus', action: () => openInline('new-folder') },
]);

const docMenuItems = computed<MenuItem[]>(() => {
  const state = contextState.value;
  if (state.kind !== 'doc') return [];
  const { slug, title } = state;
  return [
    { header: true, label: title },
    { label: 'Open', icon: 'external-link', kbd: ['↵'], action: () => emit('select-doc', slug) },
    { sep: true },
    { label: 'Rename', icon: 'pencil', kbd: ['F2'], action: () => openDocRename(slug, title) },
    { sep: true },
    {
      label: 'Delete',
      icon: 'trash',
      kbd: ['⌫'],
      danger: true,
      action: () => emit('remove-doc', slug),
    },
  ];
});

const activeMenuItems = computed<MenuItem[]>(() =>
  contextState.value.kind === 'doc' ? docMenuItems.value : rootMenuItems.value,
);

function onContextmenu(event: MouseEvent): void {
  contextState.value = { kind: 'root' };
  contextX.value = event.clientX;
  contextY.value = event.clientY;
  contextOpen.value = true;
}

function openDocMenu(event: MouseEvent, slug: string, title: string): void {
  contextState.value = { kind: 'doc', slug, title };
  contextX.value = event.clientX;
  contextY.value = event.clientY;
  contextOpen.value = true;
}

function openInline(target: InlineTarget): void {
  inlineTarget.value = target;
  inlineValue.value = '';
  void nextTick(() => {
    inlineInputRef.value?.focus();
  });
}

function openDocRename(slug: string, currentTitle: string): void {
  pendingDocRename.value = { slug };
  inlineTarget.value = null;
  inlineValue.value = currentTitle;
  void nextTick(() => {
    inlineInputRef.value?.focus();
    inlineInputRef.value?.select();
  });
}

function commitInline(): void {
  const name = inlineValue.value.trim();
  if (name === '') {
    cancelInline();
    return;
  }

  if (pendingDocRename.value !== null) {
    emit('rename-doc', pendingDocRename.value.slug, name);
  } else if (inlineTarget.value === 'new-doc') {
    emit('create-doc', name);
  } else if (inlineTarget.value === 'new-folder') {
    emit('create-folder', name);
  }

  cancelInline();
}

function cancelInline(): void {
  inlineTarget.value = null;
  pendingDocRename.value = null;
  inlineValue.value = '';
}

function onInlineKeydown(event: KeyboardEvent): void {
  if (event.key === 'Enter') {
    event.preventDefault();
    commitInline();
  } else if (event.key === 'Escape') {
    event.preventDefault();
    cancelInline();
  }
}

defineExpose({ openNewPage: () => openInline('new-doc') });
</script>

<template>
  <div style="min-height: 100%;" @contextmenu.prevent="onContextmenu">
    <div class="notes-tree-header">
      <SectionLabel>{{ projectName }}</SectionLabel>
      <button
        type="button"
        class="notes-tree-add"
        title="New page or folder"
        aria-label="New page or folder"
        @click.stop="onContextmenu"
      >
        <Icon name="more-horizontal" :size="14" />
      </button>
    </div>

    <p
      v-if="isEmpty && inlineTarget === null"
      style="padding: 8px; font-size: var(--fs-sm); color: var(--c-muted);"
    >
      No documents yet.
    </p>

    <template v-else>
      <NoteTreeRow
        v-for="folder in tree.folders"
        :key="folder.id"
        :folder="folder"
        :depth="0"
        :active-slug="activeSlug"
        @select-doc="emit('select-doc', $event)"
        @create-doc="(title, folderId) => emit('create-doc', title, folderId)"
        @rename-doc="(slug, title) => emit('rename-doc', slug, title)"
        @remove-doc="(slug) => emit('remove-doc', slug)"
        @create-folder="(name, parentId) => emit('create-folder', name, parentId)"
        @rename-folder="(folderId, name) => emit('rename-folder', folderId, name)"
        @remove-folder="(folderId) => emit('remove-folder', folderId)"
      />

      <template v-for="doc in tree.docs" :key="doc.id">
        <div
          v-if="pendingDocRename !== null && pendingDocRename.slug === doc.slug"
          style="display: flex; align-items: center; gap: 6px; padding: 3px 8px 3px 20px;"
        >
          <Icon name="file" :size="13" style="color: var(--c-muted); flex-shrink: 0;" />
          <input
            ref="inlineInputRef"
            v-model="inlineValue"
            type="text"
            placeholder="Page name…"
            style="flex: 1; height: 28px; padding: 0 6px; background: var(--c-input); border: 1px solid var(--c-border); border-radius: var(--r-sm); font-size: var(--fs-sm); font-family: var(--font-mono); color: var(--c-foreground); outline: none;"
            @keydown="onInlineKeydown"
            @blur="cancelInline"
          />
        </div>
        <Row
          v-else
          :label="doc.title"
          icon="file"
          :active="activeSlug !== null && doc.slug === activeSlug"
          :disabled="doc.slug === null"
          :menu="doc.slug !== null"
          @click="doc.slug !== null && emit('select-doc', doc.slug)"
          @menu="(event: MouseEvent) => doc.slug !== null && openDocMenu(event, doc.slug, doc.title)"
          @contextmenu.prevent.stop="(event: MouseEvent) => doc.slug !== null && openDocMenu(event, doc.slug, doc.title)"
        />
      </template>
    </template>

    <div
      v-if="inlineTarget !== null"
      style="display: flex; align-items: center; gap: 6px; padding: 3px 8px 3px 20px;"
    >
      <Icon
        :name="inlineTarget === 'new-doc' ? 'file' : 'folder'"
        :size="13"
        style="color: var(--c-muted); flex-shrink: 0;"
      />
      <input
        ref="inlineInputRef"
        v-model="inlineValue"
        type="text"
        :placeholder="inlineTarget === 'new-doc' ? 'Page name…' : 'Folder name…'"
        style="
          flex: 1;
          height: 28px;
          padding: 0 6px;
          background: var(--c-input);
          border: 1px solid var(--c-border);
          border-radius: var(--r-sm);
          font-size: var(--fs-sm);
          font-family: var(--font-mono);
          color: var(--c-foreground);
          outline: none;
        "
        @keydown="onInlineKeydown"
        @blur="cancelInline"
      />
    </div>

    <ContextMenu
      :open="contextOpen"
      :x="contextX"
      :y="contextY"
      :items="activeMenuItems"
      @close="contextOpen = false"
    />
  </div>
</template>

<style scoped>
.notes-tree-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.notes-tree-add {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  margin-right: 6px;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--c-muted);
  border-radius: var(--r-sm);
  cursor: pointer;
  opacity: 0;
}

.notes-tree-header:hover .notes-tree-add {
  opacity: 1;
}

.notes-tree-add:hover {
  background: var(--c-raised);
  color: var(--c-foreground);
}
</style>
