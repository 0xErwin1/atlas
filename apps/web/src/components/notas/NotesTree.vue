<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue';
import ContextMenu, { type MenuItem } from '@/components/ui/ContextMenu.vue';
import Icon from '@/components/ui/Icon.vue';
import Row from '@/components/ui/Row.vue';
import { useContextMenu } from '@/composables/useContextMenu';
import { useInlineEdit } from '@/composables/useInlineEdit';
import {
  type BoardInput,
  boardKey,
  buildNotesTree,
  type DocInput,
  docKey,
  type FolderInput,
  flattenVisible,
  folderAncestors,
  folderKey,
  parseNodeKey,
  type TreeNodeRef,
} from '@/lib/notesTree';
import { useTreeSelection } from '@/stores/treeSelection';
import { useUiStateStore } from '@/stores/uiState';
import NoteTreeRow from './NoteTreeRow.vue';

const props = withDefaults(
  defineProps<{
    projectName: string;
    folders: FolderInput[];
    docs: DocInput[];
    boards?: BoardInput[];
    activeSlug: string | null;
    activeBoardId?: string | null;
  }>(),
  { boards: () => [], activeBoardId: null },
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

const tree = computed(() => buildNotesTree(props.folders, props.docs, props.boards));

const selection = useTreeSelection();
const uiState = useUiStateStore();

const rootEl = ref<HTMLElement | null>(null);

// Navigating to a document (e.g. via a wikilink) reveals it in the tree: expand
// every ancestor folder, then scroll the active row into view.
function revealActive(slug: string | null): void {
  if (slug === null) return;

  const doc = props.docs.find((d) => d.slug === slug);
  if (doc === undefined) return;

  for (const folderId of folderAncestors(props.folders, doc.folder_id ?? null)) {
    if (uiState.isFolderCollapsed(folderId)) uiState.setFolderCollapsed(folderId, false);
  }

  void nextTick(() => {
    const active = rootEl.value?.querySelector('[aria-current="true"]');
    if (active instanceof HTMLElement && typeof active.scrollIntoView === 'function') {
      active.scrollIntoView({ block: 'nearest' });
    }
  });
}

// Re-run when the active doc changes and when docs first arrive (deep-link load).
watch(
  () => [props.activeSlug, props.docs.length] as const,
  () => revealActive(props.activeSlug),
  { immediate: true },
);

// Keep the selection store's range order in sync with what is actually visible.
const visibleKeys = computed(() => flattenVisible(tree.value, (id) => uiState.isFolderCollapsed(id)));
watch(visibleKeys, (keys) => selection.setOrder(keys), { immediate: true });

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

const rootDragOver = ref(false);

// Dragging a selected item drags the whole selection; otherwise just that one.
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

// Drop onto empty sidebar space: move the dragged node(s) to the project root.
function onRootDrop(event: DragEvent): void {
  rootDragOver.value = false;
  const raw = event.dataTransfer?.getData(DND_MIME);
  if (raw === undefined || raw === '') return;
  try {
    const parsed = JSON.parse(raw) as { nodes?: TreeNodeRef[] };
    if (Array.isArray(parsed.nodes) && parsed.nodes.length > 0) {
      emit('move-nodes', parsed.nodes, null);
    }
  } catch {
    // ignore malformed payloads
  }
}

const isEmpty = computed(
  () => tree.value.folders.length === 0 && tree.value.docs.length === 0 && tree.value.boards.length === 0,
);

// Shared sidebar context-menu + inline-edit logic (same composables as the tasks sidebar).
const { open: menuOpen, x: menuX, y: menuY, openAt, close: closeMenu } = useContextMenu();

type ContextState =
  | { kind: 'root' }
  | { kind: 'doc'; slug: string; title: string }
  | { kind: 'board'; boardId: string; name: string };
const contextState = ref<ContextState>({ kind: 'root' });

type EditCtx =
  | { kind: 'new-doc' }
  | { kind: 'new-folder' }
  | { kind: 'new-board' }
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
  if (ctx.kind === 'new-doc') emit('create-doc', name);
  else if (ctx.kind === 'new-folder') emit('create-folder', name);
  else if (ctx.kind === 'new-board') emit('create-board', name);
  else if (ctx.kind === 'rename-board') emit('rename-board', ctx.boardId, name);
  else emit('rename-doc', ctx.slug, name);
});

const rootMenuItems = computed<MenuItem[]>(() => [
  { label: 'New page', icon: 'file-plus', action: () => startEdit({ kind: 'new-doc' }) },
  { label: 'New board', icon: 'columns-3', action: () => startEdit({ kind: 'new-board' }) },
  { label: 'New folder', icon: 'folder-plus', action: () => startEdit({ kind: 'new-folder' }) },
]);

const docMenuItems = computed<MenuItem[]>(() => {
  const state = contextState.value;
  if (state.kind !== 'doc') return [];
  const { slug, title } = state;
  return [
    { header: true, label: title },
    { label: 'Open', icon: 'external-link', kbd: ['↵'], action: () => emit('select-doc', slug) },
    { sep: true },
    {
      label: 'Rename',
      icon: 'pencil',
      kbd: ['F2'],
      action: () => startEdit({ kind: 'rename-doc', slug }, title, true),
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
  const { boardId, name } = state;
  return [
    { header: true, label: name },
    { label: 'Open', icon: 'external-link', kbd: ['↵'], action: () => emit('select-board', boardId) },
    { sep: true },
    {
      label: 'Rename',
      icon: 'pencil',
      kbd: ['F2'],
      action: () => startEdit({ kind: 'rename-board', boardId }, name, true),
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
  return rootMenuItems.value;
});

function onContextmenu(event: MouseEvent): void {
  contextState.value = { kind: 'root' };
  openAt(event);
}

function openDocMenu(event: MouseEvent, slug: string, title: string): void {
  contextState.value = { kind: 'doc', slug, title };
  openAt(event);
}

function openBoardMenu(event: MouseEvent, boardId: string, name: string): void {
  contextState.value = { kind: 'board', boardId, name };
  openAt(event);
}

defineExpose({
  openNewPage: () => startEdit({ kind: 'new-doc' }),
  openNewBoard: () => startEdit({ kind: 'new-board' }),
  openNewFolder: () => startEdit({ kind: 'new-folder' }),
});
</script>

<template>
  <div
    ref="rootEl"
    style="min-height: 100%;"
    :class="{ 'root-drop-target': rootDragOver }"
    @contextmenu.prevent="onContextmenu"
    @dragover.prevent="rootDragOver = true"
    @dragleave.self="rootDragOver = false"
    @dragend="rootDragOver = false"
    @drop.prevent="onRootDrop"
  >
    <div class="notes-tree-header">
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
      v-if="isEmpty && editActive === null"
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

      <template v-for="doc in tree.docs" :key="doc.id">
        <div
          v-if="editActive?.kind === 'rename-doc' && editActive.slug === doc.slug"
          style="display: flex; align-items: center; gap: 6px; padding: 3px 8px 3px 20px;"
        >
          <Icon name="file" :size="13" style="color: var(--c-muted); flex-shrink: 0;" />
          <input
            ref="inputRef"
            v-model="editValue"
            type="text"
            placeholder="Page name…"
            class="notes-inline-input"
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
            :active="activeSlug !== null && doc.slug === activeSlug"
            :disabled="doc.slug === null"
            :menu="doc.slug !== null"
            @click="(event: MouseEvent) => doc.slug !== null && onDocClick(event, doc.slug)"
            @menu="(event: MouseEvent) => doc.slug !== null && openDocMenu(event, doc.slug, doc.title)"
            @contextmenu.prevent.stop="(event: MouseEvent) => doc.slug !== null && openDocMenu(event, doc.slug, doc.title)"
          />
        </div>
      </template>

      <template v-for="board in tree.boards" :key="board.id">
        <div
          v-if="editActive?.kind === 'rename-board' && editActive.boardId === board.id"
          style="display: flex; align-items: center; gap: 6px; padding: 3px 8px 3px 20px;"
        >
          <Icon name="columns-3" :size="13" style="color: var(--c-muted); flex-shrink: 0;" />
          <input
            ref="inputRef"
            v-model="editValue"
            type="text"
            placeholder="Board name…"
            class="notes-inline-input"
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
            :active="activeBoardId !== null && board.id === activeBoardId"
            :right="String(board.taskCount)"
            menu
            @click="(event: MouseEvent) => onBoardClick(event, board.id)"
            @menu="(event: MouseEvent) => openBoardMenu(event, board.id, board.name)"
            @contextmenu.prevent.stop="(event: MouseEvent) => openBoardMenu(event, board.id, board.name)"
          />
        </div>
      </template>
    </template>

    <div
      v-if="editActive?.kind === 'new-doc' || editActive?.kind === 'new-folder' || editActive?.kind === 'new-board'"
      style="display: flex; align-items: center; gap: 6px; padding: 3px 8px 3px 20px;"
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
        class="notes-inline-input"
        @keydown="onEditKeydown"
        @blur="commitEdit"
      />
    </div>

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
.notes-tree-header {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  min-height: 20px;
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

.root-drop-target {
  box-shadow: inset 0 0 0 1px var(--c-primary);
  border-radius: var(--r-sm);
}

.tree-dnd {
  border-radius: var(--r-sm);
}

.tree-dnd.selected {
  background: var(--c-selection);
}

.notes-inline-input {
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
