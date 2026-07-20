<script setup lang="ts">
import { computed, ref } from 'vue';
import ContextMenu, { type MenuItem } from '@/components/ui/ContextMenu.vue';
import Icon from '@/components/ui/Icon.vue';
import Row from '@/components/ui/Row.vue';
import { useContextMenu } from '@/composables/useContextMenu';
import { useInlineEdit } from '@/composables/useInlineEdit';
import {
  boardKey,
  docKey,
  folderKey,
  parseNodeKey,
  type TreeFolder,
  type TreeNodeRef,
} from '@/lib/notesTree';
import { useTreeSelection } from '@/stores/treeSelection';
import { useUiStateStore } from '@/stores/uiState';

const props = withDefaults(
  defineProps<{
    folder: TreeFolder;
    depth: number;
    activeSlug: string | null;
    activeBoardId?: string | null;
  }>(),
  { activeBoardId: null },
);

const emit = defineEmits<{
  'select-doc': [slug: string];
  'create-doc': [title: string, folderId?: string];
  'rename-doc': [slug: string, title: string];
  'remove-doc': [slug: string];
  'create-folder': [name: string, parentFolderId?: string];
  'rename-folder': [folderId: string, name: string];
  'remove-folder': [folderId: string];
  'select-board': [boardId: string];
  'create-board': [name: string, folderId?: string];
  'rename-board': [boardId: string, name: string];
  'remove-board': [boardId: string];
  'move-nodes': [nodes: TreeNodeRef[], targetFolderId: string | null];
  'request-move': [nodes: TreeNodeRef[]];
  'request-copy': [nodes: TreeNodeRef[]];
}>();

const uiState = useUiStateStore();

// Expand/collapse is persisted per user (server-side) so it survives refreshes
// and follows the user across devices; default is expanded.
const expanded = computed(() => !uiState.isFolderCollapsed(props.folder.id));

function toggleExpanded(): void {
  uiState.setFolderCollapsed(props.folder.id, expanded.value);
}

const selection = useTreeSelection();

function onFolderClick(event: MouseEvent): void {
  const mods = { shift: event.shiftKey, meta: event.metaKey || event.ctrlKey };
  if (selection.activate(folderKey(props.folder.id), mods) === 'default') {
    toggleExpanded();
  }
}

function onDocClick(event: MouseEvent, slug: string): void {
  const mods = { shift: event.shiftKey, meta: event.metaKey || event.ctrlKey };
  if (selection.activate(docKey(slug), mods) === 'default') {
    emit('select-doc', slug);
  }
}

function onBoardClick(event: MouseEvent, boardId: string): void {
  const mods = { shift: event.shiftKey, meta: event.metaKey || event.ctrlKey };
  if (selection.activate(boardKey(boardId), mods) === 'default') {
    emit('select-board', boardId);
  }
}

const DND_MIME = 'application/atlas-node';

const dragOver = ref(false);

// Dragging an item that is part of a multi-selection drags the whole selection;
// dragging an unselected item drags just that one.
function dragPayload(node: TreeNodeRef): TreeNodeRef[] {
  const key = node.type === 'folder' ? folderKey(node.id) : docKey(node.id);
  if (selection.isSelected(key) && selection.count > 1) {
    return selection
      .keys()
      .map(parseNodeKey)
      .filter((n): n is TreeNodeRef => n !== null);
  }
  return [node];
}

function onDragStart(node: TreeNodeRef, event: DragEvent): void {
  if (event.dataTransfer === null) return;
  event.dataTransfer.setData(DND_MIME, JSON.stringify({ nodes: dragPayload(node) }));
  event.dataTransfer.effectAllowed = 'move';
}

function parseDragNodes(event: DragEvent): TreeNodeRef[] {
  const raw = event.dataTransfer?.getData(DND_MIME);
  if (raw === undefined || raw === '') return [];
  try {
    const parsed = JSON.parse(raw) as { nodes?: TreeNodeRef[] };
    return Array.isArray(parsed.nodes) ? parsed.nodes : [];
  } catch {
    return [];
  }
}

// Drop onto this folder: re-parent the dragged node(s); a folder is never
// dropped into itself.
function onFolderDrop(event: DragEvent): void {
  dragOver.value = false;
  const nodes = parseDragNodes(event).filter((n) => !(n.type === 'folder' && n.id === props.folder.id));
  if (nodes.length > 0) emit('move-nodes', nodes, props.folder.id);
}

// Shared sidebar context-menu + inline-edit logic (same composables as the tasks sidebar).
const { open: menuOpen, x: menuX, y: menuY, openAt, close: closeMenu } = useContextMenu();

type ContextState =
  | { kind: 'folder-root' }
  | { kind: 'doc'; slug: string; currentTitle: string }
  | { kind: 'board'; boardId: string; currentName: string };
const contextState = ref<ContextState>({ kind: 'folder-root' });

type EditCtx =
  | { kind: 'new-doc' }
  | { kind: 'new-folder' }
  | { kind: 'new-board' }
  | { kind: 'rename-folder' }
  | { kind: 'rename-doc'; slug: string }
  | { kind: 'rename-board'; boardId: string };

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
  else if (ctx.kind === 'new-board') emit('create-board', name, props.folder.id);
  else if (ctx.kind === 'rename-folder') emit('rename-folder', props.folder.id, name);
  else if (ctx.kind === 'rename-board') emit('rename-board', ctx.boardId, name);
  else emit('rename-doc', ctx.slug, name);
});

// Creating inside a folder: expand it first so the input and the new item show.
function startCreate(kind: 'new-doc' | 'new-folder' | 'new-board'): void {
  uiState.setFolderCollapsed(props.folder.id, false);
  startEdit({ kind });
}

const folderMenuItems = computed<MenuItem[]>(() => {
  const self: TreeNodeRef = { type: 'folder', id: props.folder.id };
  return [
    { header: true, label: props.folder.name },
    { label: 'New page', icon: 'file-plus', action: () => startCreate('new-doc') },
    { label: 'New board', icon: 'columns-3', action: () => startCreate('new-board') },
    { label: 'New folder', icon: 'folder-plus', action: () => startCreate('new-folder') },
    { sep: true },
    {
      label: 'Rename',
      icon: 'pencil',
      kbd: ['F2'],
      action: () => startEdit({ kind: 'rename-folder' }, props.folder.name, true),
    },
    { label: 'Move to…', icon: 'arrow-right', action: () => emit('request-move', dragPayload(self)) },
    { label: 'Copy to…', icon: 'copy', action: () => emit('request-copy', dragPayload(self)) },
    { sep: true },
    {
      label: 'Delete folder',
      icon: 'trash',
      danger: true,
      action: () => emit('remove-folder', props.folder.id),
    },
  ];
});

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
    {
      label: 'Move to…',
      icon: 'arrow-right',
      action: () => emit('request-move', dragPayload({ type: 'doc', id: slug })),
    },
    {
      label: 'Copy to…',
      icon: 'copy',
      action: () => emit('request-copy', dragPayload({ type: 'doc', id: slug })),
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

const boardMenuItems = computed<MenuItem[]>(() => {
  const state = contextState.value;
  if (state.kind !== 'board') return [];

  const { boardId, currentName } = state;
  return [
    { header: true, label: currentName },
    { label: 'Open', icon: 'external-link', kbd: ['↵'], action: () => emit('select-board', boardId) },
    { sep: true },
    {
      label: 'Rename',
      icon: 'pencil',
      kbd: ['F2'],
      action: () => startEdit({ kind: 'rename-board', boardId }, currentName, true),
    },
    {
      label: 'Move to…',
      icon: 'arrow-right',
      action: () => emit('request-move', dragPayload({ type: 'board', id: boardId })),
    },
    { sep: true },
    {
      label: 'Delete',
      icon: 'trash',
      kbd: ['⌫'],
      danger: true,
      action: () => emit('remove-board', boardId),
    },
  ];
});

const activeMenuItems = computed<MenuItem[]>(() => {
  if (contextState.value.kind === 'doc') return docMenuItems.value;
  if (contextState.value.kind === 'board') return boardMenuItems.value;
  return folderMenuItems.value;
});

function openFolderMenu(event: MouseEvent): void {
  contextState.value = { kind: 'folder-root' };
  openAt(event);
}

function openDocMenu(event: MouseEvent, slug: string, title: string): void {
  contextState.value = { kind: 'doc', slug, currentTitle: title };
  openAt(event);
}

function openBoardMenu(event: MouseEvent, boardId: string, name: string): void {
  contextState.value = { kind: 'board', boardId, currentName: name };
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
      :class="{ 'drop-target': dragOver, selected: selection.isSelected(folderKey(folder.id)) }"
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
        menu-icon="plus"
        menu-label="Add page or folder"
        menu-always-visible
        @click="onFolderClick"
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
        :active-board-id="activeBoardId"
        @select-doc="emit('select-doc', $event)"
        @create-doc="(title, folderId) => emit('create-doc', title, folderId)"
        @rename-doc="(slug, title) => emit('rename-doc', slug, title)"
        @remove-doc="(slug) => emit('remove-doc', slug)"
        @create-folder="(name, parentId) => emit('create-folder', name, parentId)"
        @rename-folder="(folderId, name) => emit('rename-folder', folderId, name)"
        @remove-folder="(folderId) => emit('remove-folder', folderId)"
        @select-board="(boardId) => emit('select-board', boardId)"
        @create-board="(name, folderId) => emit('create-board', name, folderId)"
        @rename-board="(boardId, name) => emit('rename-board', boardId, name)"
        @remove-board="(boardId) => emit('remove-board', boardId)"
        @move-nodes="(nodes, target) => emit('move-nodes', nodes, target)"
        @request-move="(nodes) => emit('request-move', nodes)"
        @request-copy="(nodes) => emit('request-copy', nodes)"
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
          :class="{ selected: doc.slug !== null && selection.isSelected(docKey(doc.slug)) }"
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
            @click="(event: MouseEvent) => doc.slug !== null && onDocClick(event, doc.slug)"
            @menu="(event: MouseEvent) => doc.slug !== null && openDocMenu(event, doc.slug, doc.title)"
            @contextmenu.prevent.stop="(event: MouseEvent) => doc.slug !== null && openDocMenu(event, doc.slug, doc.title)"
          />
        </div>
      </template>

      <template v-for="board in folder.boards" :key="board.id">
        <div
          v-if="editActive?.kind === 'rename-board' && editActive.boardId === board.id"
          style="display: flex; align-items: center; gap: 6px;"
          :style="{ paddingLeft: inlinePaddingLeft, paddingRight: '8px' }"
        >
          <Icon name="columns-3" :size="13" style="color: var(--c-muted); flex-shrink: 0;" />
          <input
            ref="inputRef"
            v-model="editValue"
            type="text"
            placeholder="Board name…"
            class="note-inline-input"
            @keydown="onEditKeydown"
            @blur="commitEdit"
          />
        </div>

        <div
          v-else
          class="tree-dnd"
          :class="{ selected: selection.isSelected(boardKey(board.id)) }"
          draggable="true"
          @dragstart.stop="onDragStart({ type: 'board', id: board.id }, $event)"
        >
          <Row
            :label="board.name"
            icon="columns-3"
            :depth="depth + 1"
            :active="activeBoardId !== null && board.id === activeBoardId"
            :right="String(board.taskCount)"
            menu
            @click="(event: MouseEvent) => onBoardClick(event, board.id)"
            @menu="(event: MouseEvent) => openBoardMenu(event, board.id, board.name)"
            @contextmenu.prevent.stop="(event: MouseEvent) => openBoardMenu(event, board.id, board.name)"
          />
        </div>
      </template>

      <div
        v-if="editActive?.kind === 'new-doc' || editActive?.kind === 'new-folder' || editActive?.kind === 'new-board'"
        style="display: flex; align-items: center; gap: 6px;"
        :style="{ paddingLeft: inlinePaddingLeft, paddingRight: '8px' }"
      >
        <Icon
          :name="editActive.kind === 'new-doc' ? 'file' : editActive.kind === 'new-board' ? 'columns-3' : 'folder'"
          :size="13"
          style="color: var(--c-muted); flex-shrink: 0;"
        />
        <input
          ref="inputRef"
          v-model="editValue"
          type="text"
          :placeholder="editActive.kind === 'new-doc' ? 'Page name…' : editActive.kind === 'new-board' ? 'Board name…' : 'Folder name…'"
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

.tree-dnd.selected {
  background: var(--c-selection);
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
