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

type InlineTarget = 'new-doc' | 'new-folder';
const inlineTarget = ref<InlineTarget | null>(null);
const inlineValue = ref('');
const inlineInputRef = ref<HTMLInputElement | null>(null);

const rootMenuItems = computed<MenuItem[]>(() => [
  {
    label: 'New page',
    icon: 'file-plus',
    action: () => openInline('new-doc'),
  },
  {
    label: 'New folder',
    icon: 'folder-plus',
    action: () => openInline('new-folder'),
  },
]);

function onContextmenu(event: MouseEvent): void {
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

function commitInline(): void {
  const name = inlineValue.value.trim();
  if (name === '') {
    cancelInline();
    return;
  }

  if (inlineTarget.value === 'new-doc') {
    emit('create-doc', name);
  } else if (inlineTarget.value === 'new-folder') {
    emit('create-folder', name);
  }

  inlineTarget.value = null;
  inlineValue.value = '';
}

function cancelInline(): void {
  inlineTarget.value = null;
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
    <SectionLabel>{{ projectName }}</SectionLabel>

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

      <Row
        v-for="doc in tree.docs"
        :key="doc.id"
        :label="doc.title"
        icon="file"
        :active="activeSlug !== null && doc.slug === activeSlug"
        :disabled="doc.slug === null"
        @click="doc.slug !== null && emit('select-doc', doc.slug)"
        @contextmenu.prevent.stop="(event: MouseEvent) => {
          contextX = event.clientX;
          contextY = event.clientY;
          contextOpen = true;
        }"
      />
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
      :items="rootMenuItems"
      @close="contextOpen = false"
    />
  </div>
</template>
