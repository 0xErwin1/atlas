<script setup lang="ts">
import { computed, ref } from 'vue';
import ContextMenu, { type MenuItem } from '@/components/ui/ContextMenu.vue';
import Icon from '@/components/ui/Icon.vue';
import Row from '@/components/ui/Row.vue';
import { useContextMenu } from '@/composables/useContextMenu';
import { useInlineEdit } from '@/composables/useInlineEdit';
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
  'move-doc': [slug: string, targetFolderId: string | null];
  'move-folder': [folderId: string, targetParentId: string | null];
}>();

const expanded = ref(true);

const DND_MIME = 'application/atlas-node';

const dragOver = ref(false);

interface DragNode {
  type: 'doc' | 'folder';
  id: string;
}

function onDragStart(node: DragNode, event: DragEvent): void {
  if (event.dataTransfer === null) return;
  event.dataTransfer.setData(DND_MIME, JSON.stringify(node));
  event.dataTransfer.effectAllowed = 'move';
}

function parseDragNode(event: DragEvent): DragNode | null {
  const raw = event.dataTransfer?.getData(DND_MIME);
  if (raw === undefined || raw === '') return null;
  try {
    const node = JSON.parse(raw) as DragNode;
    return node.type === 'doc' || node.type === 'folder' ? node : null;
  } catch {
    return null;
  }
}

// Drop onto this folder: re-parent a document, or a folder (never onto itself).
function onFolderDrop(event: DragEvent): void {
  dragOver.value = false;
  const node = parseDragNode(event);
  if (node === null) return;
  if (node.type === 'doc') emit('move-doc', node.id, props.folder.id);
  else if (node.id !== props.folder.id) emit('move-folder', node.id, props.folder.id);
}

// Shared sidebar context-menu + inline-edit logic (same composables as the tasks sidebar).
const { open: menuOpen, x: menuX, y: menuY, openAt, close: closeMenu } = useContextMenu();

type ContextState = { kind: 'folder-root' } | { kind: 'doc'; slug: string; currentTitle: string };
const contextState = ref<ContextState>({ kind: 'folder-root' });

type EditCtx =
  | { kind: 'new-doc' }
  | { kind: 'new-folder' }
  | { kind: 'rename-folder' }
  | { kind: 'rename-doc'; slug: string };

const {
  active: editActive,
  value: editValue,
  inputRef,
  start: startEdit,
  commit: commitEdit,
  onKeydown: onEditKeydown,
} = useInlineEdit<EditCtx>((name, ctx) => {
  if (ctx.kind === 'new-doc') emit('create-doc', name, props.folder.id);
  else if (ctx.kind === 'new-folder') emit('create-folder', name, props.folder.id);
  else if (ctx.kind === 'rename-folder') emit('rename-folder', props.folder.id, name);
  else emit('rename-doc', ctx.slug, name);
});

// Creating inside a folder: expand it first so the input and the new item show.
function startCreate(kind: 'new-doc' | 'new-folder'): void {
  expanded.value = true;
  startEdit({ kind });
}

const folderMenuItems = computed<MenuItem[]>(() => [
  { header: true, label: props.folder.name },
  { label: 'New page', icon: 'file-plus', action: () => startCreate('new-doc') },
  { label: 'New folder', icon: 'folder-plus', action: () => startCreate('new-folder') },
  { sep: true },
  {
    label: 'Rename',
    icon: 'pencil',
    kbd: ['F2'],
    action: () => startEdit({ kind: 'rename-folder' }, props.folder.name, true),
  },
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

  const { slug, currentTitle } = state;
  return [
    { header: true, label: currentTitle },
    { label: 'Open', icon: 'external-link', kbd: ['↵'], action: () => emit('select-doc', slug) },
    { sep: true },
    {
      label: 'Rename',
      icon: 'pencil',
      kbd: ['F2'],
      action: () => startEdit({ kind: 'rename-doc', slug }, currentTitle, true),
    },
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
  openAt(event);
}

function openDocMenu(event: MouseEvent, slug: string, title: string): void {
  contextState.value = { kind: 'doc', slug, currentTitle: title };
  openAt(event);
}

const isRenamingFolder = computed(() => editActive.value?.kind === 'rename-folder');
const inlinePaddingLeft = computed(() => `${8 + (props.depth + 1) * 14}px`);
</script>

<template>
  <div>
    <div
      v-if="isRenamingFolder"
      style="display: flex; align-items: center; gap: 6px;"
      :style="{ paddingLeft: `${8 + depth * 14}px`, paddingRight: '8px' }"
    >
      <Icon name="folder" :size="13" style="color: var(--c-muted); flex-shrink: 0;" />
      <input
        ref="inputRef"
        v-model="editValue"
        type="text"
        placeholder="Folder name…"
        class="note-inline-input"
        @keydown="onEditKeydown"
        @blur="commitEdit"
      />
    </div>

    <div
      v-else
      draggable="true"
      class="tree-dnd"
      :class="{ 'drop-target': dragOver }"
      @dragstart.stop="onDragStart({ type: 'folder', id: folder.id }, $event)"
      @dragover.prevent="dragOver = true"
      @dragenter.prevent="dragOver = true"
      @dragleave="dragOver = false"
      @drop.prevent.stop="onFolderDrop"
    >
      <Row
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
    </div>

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
        @move-doc="(slug, target) => emit('move-doc', slug, target)"
        @move-folder="(id, target) => emit('move-folder', id, target)"
      />

      <template v-for="doc in folder.docs" :key="doc.id">
        <div
          v-if="editActive?.kind === 'rename-doc' && editActive.slug === doc.slug"
          style="display: flex; align-items: center; gap: 6px;"
          :style="{ paddingLeft: inlinePaddingLeft, paddingRight: '8px' }"
        >
          <Icon name="file" :size="13" style="color: var(--c-muted); flex-shrink: 0;" />
          <input
            ref="inputRef"
            v-model="editValue"
            type="text"
            placeholder="Page name…"
            class="note-inline-input"
            @keydown="onEditKeydown"
            @blur="commitEdit"
          />
        </div>

        <div
          v-else
          class="tree-dnd"
          :draggable="doc.slug !== null"
          @dragstart.stop="doc.slug !== null && onDragStart({ type: 'doc', id: doc.slug }, $event)"
        >
          <Row
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
        </div>
      </template>

      <div
        v-if="editActive?.kind === 'new-doc' || editActive?.kind === 'new-folder'"
        style="display: flex; align-items: center; gap: 6px;"
        :style="{ paddingLeft: inlinePaddingLeft, paddingRight: '8px' }"
      >
        <Icon
          :name="editActive.kind === 'new-doc' ? 'file' : 'folder'"
          :size="13"
          style="color: var(--c-muted); flex-shrink: 0;"
        />
        <input
          ref="inputRef"
          v-model="editValue"
          type="text"
          :placeholder="editActive.kind === 'new-doc' ? 'Page name…' : 'Folder name…'"
          class="note-inline-input"
          @keydown="onEditKeydown"
          @blur="commitEdit"
        />
      </div>
    </template>

    <ContextMenu
      :open="menuOpen"
      :x="menuX"
      :y="menuY"
      :items="activeMenuItems"
      @close="closeMenu"
    />
  </div>
</template>

<style scoped>
.tree-dnd {
  border-radius: var(--r-sm);
}

.tree-dnd.drop-target {
  background: var(--c-selection);
  box-shadow: inset 0 0 0 1px var(--c-primary);
}

.note-inline-input {
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
}
</style>
