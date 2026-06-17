<script setup lang="ts">
import { computed, nextTick, ref } from 'vue';
import ContextMenu, { type MenuItem } from '@/components/ui/ContextMenu.vue';
import Icon from '@/components/ui/Icon.vue';
import Row from '@/components/ui/Row.vue';
import type { TreeFolder } from '@/lib/notesTree';

const props = defineProps<{
  folder: TreeFolder;
  depth: number;
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

const expanded = ref(true);

const contextOpen = ref(false);
const contextX = ref(0);
const contextY = ref(0);

type DocContextState = { kind: 'folder-root' } | { kind: 'doc'; slug: string; currentTitle: string };
const contextState = ref<DocContextState>({ kind: 'folder-root' });

type InlineMode = 'new-doc' | 'new-folder' | 'rename-folder';
const inlineMode = ref<InlineMode | null>(null);
const inlineValue = ref('');
const inlineInputRef = ref<HTMLInputElement | null>(null);

const folderMenuItems = computed<MenuItem[]>(() => [
  { header: true, label: props.folder.name },
  { label: 'New page', icon: 'file-plus', action: () => openInline('new-doc') },
  { label: 'New folder', icon: 'folder-plus', action: () => openInline('new-folder') },
  { sep: true },
  { label: 'Rename', icon: 'pencil', kbd: ['F2'], action: () => openInline('rename-folder') },
  { sep: true },
  {
    label: 'Delete folder',
    icon: 'trash',
    danger: true,
    action: () => emit('remove-folder', props.folder.id),
  },
]);

const docMenuItems = computed<MenuItem[]>(() => {
  const state = contextState.value;
  if (state.kind !== 'doc') return [];

  const slug = state.slug;
  const currentTitle = state.currentTitle;

  return [
    { header: true, label: currentTitle },
    { label: 'Open', icon: 'external-link', kbd: ['↵'], action: () => emit('select-doc', slug) },
    { sep: true },
    { label: 'Rename', icon: 'pencil', kbd: ['F2'], action: () => openDocRename(slug, currentTitle) },
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
  contextState.value.kind === 'doc' ? docMenuItems.value : folderMenuItems.value,
);

function openFolderMenu(event: MouseEvent): void {
  contextState.value = { kind: 'folder-root' };
  contextX.value = event.clientX;
  contextY.value = event.clientY;
  contextOpen.value = true;
}

function openDocMenu(event: MouseEvent, slug: string, title: string): void {
  contextState.value = { kind: 'doc', slug, currentTitle: title };
  contextX.value = event.clientX;
  contextY.value = event.clientY;
  contextOpen.value = true;
}

type DocRenameState = { slug: string } | null;
const pendingDocRename = ref<DocRenameState>(null);

function openInline(mode: InlineMode): void {
  inlineMode.value = mode;
  if (mode === 'rename-folder') {
    inlineValue.value = props.folder.name;
  } else {
    inlineValue.value = '';
  }
  void nextTick(() => {
    inlineInputRef.value?.focus();
    if (mode === 'rename-folder') {
      inlineInputRef.value?.select();
    }
  });
}

function openDocRename(slug: string, currentTitle: string): void {
  pendingDocRename.value = { slug };
  inlineMode.value = null;
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
    pendingDocRename.value = null;
    inlineValue.value = '';
    return;
  }

  const mode = inlineMode.value;
  if (mode === 'new-doc') {
    emit('create-doc', name, props.folder.id);
  } else if (mode === 'new-folder') {
    emit('create-folder', name, props.folder.id);
  } else if (mode === 'rename-folder') {
    emit('rename-folder', props.folder.id, name);
  }

  inlineMode.value = null;
  inlineValue.value = '';
}

function cancelInline(): void {
  inlineMode.value = null;
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

const isRenaming = computed(() => inlineMode.value === 'rename-folder');
const isCreatingDoc = computed(() => inlineMode.value === 'new-doc');
const isCreatingFolder = computed(() => inlineMode.value === 'new-folder');
const isRenamingDoc = computed(() => pendingDocRename.value !== null);

const inlinePaddingLeft = computed(() => `${8 + (props.depth + 1) * 14}px`);
</script>

<template>
  <div>
    <div
      v-if="isRenaming"
      style="display: flex; align-items: center; gap: 6px;"
      :style="{ paddingLeft: `${8 + depth * 14}px`, paddingRight: '8px' }"
    >
      <Icon name="folder" :size="13" style="color: var(--c-muted); flex-shrink: 0;" />
      <input
        ref="inlineInputRef"
        v-model="inlineValue"
        type="text"
        placeholder="Folder name…"
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

    <Row
      v-else
      :label="folder.name"
      :icon="expanded ? 'folder-open' : 'folder'"
      :depth="depth"
      chevron
      :open="expanded"
      menu
      @click="expanded = !expanded"
      @menu="openFolderMenu"
      @contextmenu.prevent.stop="openFolderMenu"
    />

    <template v-if="expanded">
      <NoteTreeRow
        v-for="child in folder.folders"
        :key="child.id"
        :folder="child"
        :depth="depth + 1"
        :active-slug="activeSlug"
        @select-doc="emit('select-doc', $event)"
        @create-doc="(title, folderId) => emit('create-doc', title, folderId)"
        @rename-doc="(slug, title) => emit('rename-doc', slug, title)"
        @remove-doc="(slug) => emit('remove-doc', slug)"
        @create-folder="(name, parentId) => emit('create-folder', name, parentId)"
        @rename-folder="(folderId, name) => emit('rename-folder', folderId, name)"
        @remove-folder="(folderId) => emit('remove-folder', folderId)"
      />

      <template v-for="doc in folder.docs" :key="doc.id">
        <div
          v-if="isRenamingDoc && pendingDocRename?.slug === doc.slug"
          style="display: flex; align-items: center; gap: 6px;"
          :style="{ paddingLeft: inlinePaddingLeft, paddingRight: '8px' }"
        >
          <Icon name="file" :size="13" style="color: var(--c-muted); flex-shrink: 0;" />
          <input
            ref="inlineInputRef"
            v-model="inlineValue"
            type="text"
            placeholder="Page name…"
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

        <Row
          v-else
          :label="doc.title"
          icon="file"
          :depth="depth + 1"
          :active="activeSlug !== null && doc.slug === activeSlug"
          :disabled="doc.slug === null"
          :menu="doc.slug !== null"
          @click="doc.slug !== null && emit('select-doc', doc.slug)"
          @menu="(event: MouseEvent) => doc.slug !== null && openDocMenu(event, doc.slug, doc.title)"
          @contextmenu.prevent.stop="(event: MouseEvent) => doc.slug !== null && openDocMenu(event, doc.slug, doc.title)"
        />
      </template>

      <div
        v-if="isCreatingDoc || isCreatingFolder"
        style="display: flex; align-items: center; gap: 6px;"
        :style="{ paddingLeft: inlinePaddingLeft, paddingRight: '8px' }"
      >
        <Icon
          :name="isCreatingDoc ? 'file' : 'folder'"
          :size="13"
          style="color: var(--c-muted); flex-shrink: 0;"
        />
        <input
          ref="inlineInputRef"
          v-model="inlineValue"
          type="text"
          :placeholder="isCreatingDoc ? 'Page name…' : 'Folder name…'"
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
    </template>

    <ContextMenu
      :open="contextOpen"
      :x="contextX"
      :y="contextY"
      :items="activeMenuItems"
      @close="contextOpen = false"
    />
  </div>
</template>
