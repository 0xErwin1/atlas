<script setup lang="ts">
import { computed, nextTick, ref } from 'vue';
import { VueDraggable } from 'vue-draggable-plus';
import TaskCard from '@/components/tareas/TaskCard.vue';
import Btn from '@/components/ui/Btn.vue';
import ColorPicker from '@/components/ui/ColorPicker.vue';
import ContextMenu, { type MenuItem } from '@/components/ui/ContextMenu.vue';
import Icon from '@/components/ui/Icon.vue';
import { resolveDropTarget } from '@/composables/kanbanDrop';
import { useContextMenu } from '@/composables/useContextMenu';
import { useInlineEdit } from '@/composables/useInlineEdit';
import { resolveColumnSwatchId } from '@/lib/columnColor';
import { swatchById } from '@/lib/swatches';
import type { ColumnDto, TaskSummaryDto } from '@/stores/boards';

const props = defineProps<{
  column: ColumnDto;
  tasks: TaskSummaryDto[];
  selectedReadableId?: string | null;
  // On mobile the column fills most of the viewport and snaps, with the next
  // column peeking, instead of the fixed 250px desktop width.
  fluid?: boolean;
  // Position within the board, used to disable move-left/right at the edges.
  isFirst?: boolean;
  isLast?: boolean;
}>();

const emit = defineEmits<{
  /** A drop landed in this column: (readableId, columnId, toIndex). */
  drop: [readableId: string, columnId: string, toIndex: number];
  /** Quick-add: create a task in this column with the given title. */
  create: [columnId: string, title: string];
  select: [readableId: string];
  open: [readableId: string];
  menu: [readableId: string, event: MouseEvent];
  /** Persist an edited name and/or color for this column. */
  'save-column': [column: ColumnDto, draft: { name: string; color: string }];
  /** Reorder this column one slot (-1 left, +1 right). */
  'move-column': [column: ColumnDto, direction: -1 | 1];
  /** Request deletion of this column (parent owns the confirm dialog). */
  'delete-column': [column: ColumnDto];
}>();

const {
  active: adding,
  value: addValue,
  inputRef,
  start: startAdd,
  commit: commitAdd,
  onKeydown: onAddKeydown,
} = useInlineEdit<'task'>((title) => emit('create', props.column.id, title));

const dotColor = computed(() => swatchById(resolveColumnSwatchId(props.column)).fg);

const menu = useContextMenu();

const editing = ref(false);
const draftName = ref('');
const draftColor = ref('');
const renameRef = ref<HTMLInputElement | null>(null);

const draftDotColor = computed(() => swatchById(draftColor.value).fg);

function openMenu(event: MouseEvent): void {
  menu.openAt(event);
}

const menuItems = computed<MenuItem[]>(() => [
  { label: 'Edit name & color', icon: 'pencil', action: startEdit },
  {
    label: 'Move left',
    icon: 'chevron-left',
    disabled: props.isFirst === true,
    action: () => emit('move-column', props.column, -1),
  },
  {
    label: 'Move right',
    icon: 'chevron-right',
    disabled: props.isLast === true,
    action: () => emit('move-column', props.column, 1),
  },
  { sep: true },
  { label: 'Delete status', icon: 'trash', danger: true, action: () => emit('delete-column', props.column) },
]);

async function startEdit(): Promise<void> {
  draftName.value = props.column.name;
  draftColor.value = resolveColumnSwatchId(props.column);
  editing.value = true;

  await nextTick();
  renameRef.value?.focus();
  renameRef.value?.select();
}

function cancelEdit(): void {
  editing.value = false;
}

/**
 * Emits the edited draft and leaves edit mode immediately; the parent persists
 * the changed fields and re-renders the header from the store on success.
 */
function saveEdit(): void {
  emit('save-column', props.column, { name: draftName.value, color: draftColor.value });
  editing.value = false;
}

/**
 * vue-draggable-plus drives `v-model` mutation on drop; we ignore the mutated
 * model and instead translate the SortableJS change event into a move command.
 * The store (via useKanbanMove) owns the authoritative reordering, so the local
 * model is treated as ephemeral.
 */
const model = computed({
  get: () => props.tasks,
  set: () => undefined,
});

function onSortableDrop(event: unknown): void {
  const target = resolveDropTarget(event as Parameters<typeof resolveDropTarget>[0]);
  if (target === null) {
    return;
  }
  emit('drop', target.readableId, props.column.id, target.toIndex);
}
</script>

<template>
  <div
    class="flex flex-col min-h-0"
    :style="fluid
      ? 'width: 84vw; max-width: 320px; flex: 0 0 84vw; scroll-snap-align: start;'
      : 'width: 250px; flex: 0 0 250px;'"
  >
    <div
      v-if="!editing"
      class="atl-col-head flex items-center"
      style="gap: 7px; padding: 0 2px 9px;"
    >
      <span
        :style="{
          width: '7px',
          height: '7px',
          borderRadius: 'var(--r-full)',
          backgroundColor: dotColor,
          flex: '0 0 auto',
        }"
        aria-hidden="true"
      />
      <span
        style="font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--c-foreground); cursor: text;"
        title="Double-click to edit"
        @dblclick="startEdit"
      >
        {{ column.name }}
      </span>
      <span
        style="font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--c-muted);"
      >
        {{ tasks.length }}
      </span>
      <span class="flex-1" />
      <button
        type="button"
        class="atl-gbtn atl-col-menu"
        title="Edit status"
        aria-label="Edit status"
        style="width: 20px; height: 20px; min-width: 20px; padding: 0;"
        @click="openMenu"
      >
        <Icon name="ellipsis" :size="14" />
      </button>
      <button
        type="button"
        class="atl-gbtn"
        title="Add task"
        aria-label="Add task"
        style="width: 20px; height: 20px; min-width: 20px; padding: 0;"
        @click="startAdd('task')"
      >
        <Icon name="plus" :size="13" />
      </button>
    </div>

    <div v-else class="atl-col-edit" style="padding: 0 2px 9px;">
      <div class="flex items-center" style="gap: 7px;">
        <span
          :style="{
            width: '7px',
            height: '7px',
            borderRadius: 'var(--r-full)',
            backgroundColor: draftDotColor,
            flex: '0 0 auto',
          }"
          aria-hidden="true"
        />
        <input
          ref="renameRef"
          v-model="draftName"
          type="text"
          class="atl-col-rename"
          aria-label="Status name"
          @keydown.enter="saveEdit"
          @keydown.esc="cancelEdit"
        />
      </div>

      <ColorPicker
        class="atl-col-picker"
        :selected="draftColor"
        @select="(id) => { draftColor = id; }"
      />

      <div class="flex items-center" style="gap: 6px;">
        <Btn variant="primary" @click="saveEdit">Save</Btn>
        <button type="button" class="atl-col-cancel" @click="cancelEdit">Cancel</button>
      </div>
    </div>

    <div v-if="adding !== null" style="margin-bottom: 8px;">
      <input
        ref="inputRef"
        v-model="addValue"
        type="text"
        placeholder="Task title…"
        class="atl-quick-add"
        @keydown="onAddKeydown"
        @blur="commitAdd"
      />
    </div>

    <VueDraggable
      v-model="model"
      :group="'kanban'"
      :animation="150"
      item-key="id"
      class="flex flex-col"
      style="gap: 8px; flex: 1 1 auto; min-height: 60px;"
      ghost-class="atl-card-ghost"
      @add="onSortableDrop"
      @update="onSortableDrop"
    >
      <TaskCard
        v-for="task in tasks"
        :key="task.id"
        :task="task"
        :selected="task.readable_id === selectedReadableId"
        @select="(id) => emit('select', id)"
        @open="(id) => emit('open', id)"
        @menu="(id, event) => emit('menu', id, event)"
      />
    </VueDraggable>

    <ContextMenu
      :open="menu.open.value"
      :x="menu.x.value"
      :y="menu.y.value"
      :items="menuItems"
      @close="menu.close"
    />
  </div>
</template>

<style scoped>
.atl-card-ghost {
  opacity: 0.4;
}

.atl-col-menu {
  opacity: 0;
  transition: opacity 0.12s;
}

.atl-col-head:hover .atl-col-menu,
.atl-col-menu:focus-visible {
  opacity: 1;
}

.atl-col-edit {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-bottom: 8px;
}

.atl-col-picker {
  align-self: flex-start;
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  background: var(--c-raised);
}

.atl-col-rename {
  flex: 1 1 auto;
  min-width: 0;
  height: 28px;
  padding: 0 8px;
  background: var(--c-raised);
  border: 1px solid var(--c-primary);
  border-radius: var(--r-md);
  font-size: var(--fs-sm);
  font-weight: var(--fw-semibold);
  color: var(--c-foreground);
  outline: none;
}

.atl-col-cancel {
  height: var(--h-button);
  padding: 0 8px;
  background: transparent;
  border: none;
  color: var(--c-muted);
  font-size: var(--fs-sm);
  cursor: pointer;
}

.atl-col-cancel:hover {
  color: var(--c-foreground);
}

.atl-quick-add {
  width: 100%;
  height: 32px;
  padding: 0 9px;
  background: var(--c-raised);
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  font-size: 12.5px;
  font-family: var(--font-mono);
  color: var(--c-foreground);
  outline: none;
}

.atl-quick-add:focus {
  border-color: var(--c-primary);
}
</style>
