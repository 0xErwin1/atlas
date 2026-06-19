<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import ContextMenu, { type MenuItem } from '@/components/ui/ContextMenu.vue';
import Icon from '@/components/ui/Icon.vue';
import Row from '@/components/ui/Row.vue';
import { useContextMenu } from '@/composables/useContextMenu';
import { useInlineEdit } from '@/composables/useInlineEdit';
import { buildNotesTree, type DocInput, docKey, type FolderInput, flattenVisible } from '@/lib/notesTree';
import { useTreeSelection } from '@/stores/treeSelection';
import { useUiStateStore } from '@/stores/uiState';
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
  'move-doc': [slug: string, targetFolderId: string | null];
  'move-folder': [folderId: string, targetParentId: string | null];
}>();

const tree = computed(() => buildNotesTree(props.folders, props.docs));

const selection = useTreeSelection();
const uiState = useUiStateStore();

// Keep the selection store's range order in sync with what is actually visible.
const visibleKeys = computed(() => flattenVisible(tree.value, (id) => uiState.isFolderCollapsed(id)));
watch(visibleKeys, (keys) => selection.setOrder(keys), { immediate: true });

function onDocClick(event: MouseEvent, slug: string): void {
  const mods = { shift: event.shiftKey, meta: event.metaKey || event.ctrlKey };
  if (selection.activate(docKey(slug), mods) === 'default') {
    emit('select-doc', slug);
  }
}

const DND_MIME = 'application/atlas-node';

interface DragNode {
  type: 'doc' | 'folder';
  id: string;
}

const rootDragOver = ref(false);

function onDragStart(node: DragNode, event: DragEvent): void {
  if (event.dataTransfer === null) return;
  event.dataTransfer.setData(DND_MIME, JSON.stringify(node));
  event.dataTransfer.effectAllowed = 'move';
}

// Drop onto empty sidebar space: move the dragged node to the project root.
function onRootDrop(event: DragEvent): void {
  rootDragOver.value = false;
  const raw = event.dataTransfer?.getData(DND_MIME);
  if (raw === undefined || raw === '') return;
  let node: DragNode;
  try {
    node = JSON.parse(raw) as DragNode;
  } catch {
    return;
  }
  if (node.type === 'doc') emit('move-doc', node.id, null);
  else if (node.type === 'folder') emit('move-folder', node.id, null);
}

const isEmpty = computed(() => tree.value.folders.length === 0 && tree.value.docs.length === 0);

// Shared sidebar context-menu + inline-edit logic (same composables as the tasks sidebar).
const { open: menuOpen, x: menuX, y: menuY, openAt, close: closeMenu } = useContextMenu();

type ContextState = { kind: 'root' } | { kind: 'doc'; slug: string; title: string };
const contextState = ref<ContextState>({ kind: 'root' });

type EditCtx = { kind: 'new-doc' } | { kind: 'new-folder' } | { kind: 'rename-doc'; slug: string };

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
  else emit('rename-doc', ctx.slug, name);
});

const rootMenuItems = computed<MenuItem[]>(() => [
  { label: 'New page', icon: 'file-plus', action: () => startEdit({ kind: 'new-doc' }) },
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
  openAt(event);
}

function openDocMenu(event: MouseEvent, slug: string, title: string): void {
  contextState.value = { kind: 'doc', slug, title };
  openAt(event);
}

defineExpose({ openNewPage: () => startEdit({ kind: 'new-doc' }) });
</script>

<template>
  <div
    style="min-height: 100%;"
    :class="{ 'root-drop-target': rootDragOver }"
    @contextmenu.prevent="onContextmenu"
    @dragover.prevent="rootDragOver = true"
    @dragleave.self="rootDragOver = false"
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
    </template>

    <div
      v-if="editActive?.kind === 'new-doc' || editActive?.kind === 'new-folder'"
      style="display: flex; align-items: center; gap: 6px; padding: 3px 8px 3px 20px;"
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
