<script setup lang="ts">
import { computed, ref } from 'vue';
import KanbanColumn from '@/components/tareas/KanbanColumn.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import ContextMenu from '@/components/ui/ContextMenu.vue';
import Icon from '@/components/ui/Icon.vue';
import PromptDialog from '@/components/ui/PromptDialog.vue';
import { useBreakpoint } from '@/composables/useBreakpoint';
import { useContextMenu } from '@/composables/useContextMenu';
import { useKanbanMove } from '@/composables/useKanbanMove';
import { useTaskInteractions } from '@/composables/useTaskInteractions';
import { activeDotIndex, dotScrollTarget } from '@/lib/kanbanDots';
import { useBoardsStore } from '@/stores/boards';
import { type TaskViewMode, useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

const props = defineProps<{
  ws: string;
  selectedReadableId?: string | null;
}>();

const emit = defineEmits<{
  select: [readableId: string, mode?: TaskViewMode];
  open: [readableId: string];
}>();

const boards = useBoardsStore();
const workspace = useWorkspaceStore();
const ui = useUiStore();
const { move } = useKanbanMove(props.ws);
const { isMobile } = useBreakpoint();
const ti = useTaskInteractions(props.ws);

const scrollEl = ref<HTMLElement | null>(null);
const activeColumn = ref(0);
const addColumnOpen = ref(false);

const menu = useContextMenu();

function onBoardScroll(): void {
  const el = scrollEl.value;
  if (el === null) return;
  activeColumn.value = activeDotIndex(el.scrollLeft, el.scrollWidth - el.clientWidth, boards.columns.length);
}

function scrollToColumn(index: number): void {
  const el = scrollEl.value;
  if (el === null) return;
  el.scrollTo({
    left: dotScrollTarget(index, el.scrollWidth - el.clientWidth, boards.columns.length),
    behavior: 'smooth',
  });
}

async function onDrop(readableId: string, columnId: string, toIndex: number): Promise<void> {
  const result = await move(readableId, columnId, toIndex);
  if (!result.ok) {
    ui.showBanner(result.hint ?? 'Move failed', 'error');
  }
}

async function onCreate(columnId: string, title: string): Promise<void> {
  const boardId = boards.board?.id;
  if (boardId === undefined) return;

  const created = await boards.createTask(props.ws, boardId, columnId, title);
  if (created === null && boards.error) {
    ui.showBanner(boards.error, 'error');
  }
}

async function onMenu(readableId: string, event: MouseEvent): Promise<void> {
  ti.menuReadableId.value = readableId;
  menu.openAt(event);

  void workspace.loadMembers(props.ws);
  await Promise.all(workspace.projects.map((p) => boards.loadBoardsForProject(props.ws, p.slug)));
}

async function onAddColumnConfirm(value: string): Promise<void> {
  addColumnOpen.value = false;

  const name = value.trim();
  const boardId = boards.board?.id;
  if (name.length === 0 || boardId === undefined) return;

  const created = await boards.createColumn(props.ws, boardId, name);
  if (created === null && boards.error) ui.showBanner(boards.error, 'error');
}

const deleteTarget = computed(() => ti.deleteTargetFor(ti.menuReadableId.value));

const menuItems = computed(() => {
  const readableId = ti.menuReadableId.value;
  if (readableId === null) return [];

  const task = boards.findTaskByReadableId(readableId);
  if (task === undefined) return [];

  const boardId = boards.board?.id;
  return ti.buildMenuItems({
    task,
    boardId,
    columns: boards.columns,
    allowDuplicate: boardId !== undefined,
    onOpen: (rid) => emit('open', rid),
    onOpenAs: (rid, mode) => emit('select', rid, mode),
  });
});
</script>

<template>
  <div class="flex flex-col flex-1 min-h-0 min-w-0" style="background-color: var(--c-background);">
    <div
      ref="scrollEl"
      class="flex flex-1 overflow-x-auto min-w-0"
      :style="`gap: 14px; padding: 16px; ${isMobile ? 'scroll-snap-type: x mandatory; scroll-padding-left: 16px;' : ''}`"
      @scroll="onBoardScroll"
    >
      <KanbanColumn
        v-for="column in boards.columns"
        :key="column.id"
        :column="column"
        :tasks="boards.filteredTasksByColumn(column.id)"
        :selected-readable-id="selectedReadableId"
        :fluid="isMobile"
        @drop="onDrop"
        @create="onCreate"
        @select="(id) => emit('select', id)"
        @open="(id) => emit('open', id)"
        @menu="onMenu"
      />

      <button
        v-if="boards.board?.id !== undefined && !boards.loading"
        type="button"
        class="atl-add-column"
        :style="isMobile ? 'width: 84vw; max-width: 320px; flex: 0 0 84vw; scroll-snap-align: start;' : 'width: 250px; flex: 0 0 250px;'"
        @click="addColumnOpen = true"
      >
        <Icon name="plus" :size="15" />
        Add status
      </button>

      <p
        v-if="boards.columns.length === 0 && !boards.loading"
        style="font-size: var(--fs-sm); color: var(--c-muted); padding: 8px;"
      >
        This board has no columns yet.
      </p>
    </div>

    <div
      v-if="isMobile && boards.columns.length > 1"
      class="flex items-center justify-center"
      style="gap: 7px; padding: 8px 0 10px;"
      aria-hidden="true"
    >
      <button
        v-for="(column, i) in boards.columns"
        :key="column.id"
        type="button"
        :aria-label="`Go to ${column.name}`"
        :style="`
          width: ${i === activeColumn ? '18px' : '7px'};
          height: 7px;
          border: none;
          padding: 0;
          border-radius: 9999px;
          cursor: pointer;
          background: ${i === activeColumn ? 'var(--c-primary)' : 'var(--c-border)'};
          transition: width 0.18s, background 0.18s;
        `"
        @click="scrollToColumn(i)"
      />
    </div>

    <ContextMenu
      :open="menu.open.value"
      :x="menu.x.value"
      :y="menu.y.value"
      :items="menuItems"
      @close="menu.close"
    />

    <PromptDialog
      :open="ti.promptState.value.open"
      :title="ti.promptState.value.title"
      :initial="ti.promptState.value.initial"
      :input-type="ti.promptState.value.mode === 'due' ? 'date' : 'text'"
      @confirm="ti.onPromptConfirm"
      @cancel="ti.promptState.value = { ...ti.promptState.value, open: false }"
    />

    <PromptDialog
      :open="addColumnOpen"
      title="New status"
      placeholder="Status name"
      confirm-label="Create status"
      @confirm="onAddColumnConfirm"
      @cancel="addColumnOpen = false"
    />

    <ConfirmDialog
      :open="ti.confirmOpen.value"
      tone="danger"
      title="Delete this task?"
      message="The task is removed permanently. This can't be undone."
      :detail="deleteTarget ? `${deleteTarget.readable_id} · ${deleteTarget.title}` : undefined"
      detail-icon="square-kanban"
      note="Its sub-tasks, references, and activity are removed along with it."
      confirm-label="Delete task"
      confirm-icon="trash-2"
      @confirm="ti.onConfirmDelete"
      @cancel="ti.confirmOpen.value = false"
    />
  </div>
</template>

<style scoped>
.atl-add-column {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  align-self: flex-start;
  height: 38px;
  padding: 0 12px;
  border: 1px dashed var(--c-border);
  border-radius: var(--r-lg);
  background: transparent;
  color: var(--c-muted);
  font-family: var(--font-ui);
  font-size: var(--fs-sm);
  cursor: pointer;
  transition:
    color 0.12s,
    border-color 0.12s,
    background 0.12s;
}

.atl-add-column:hover {
  color: var(--c-foreground);
  border-color: var(--c-primary);
  background: var(--c-raised);
}
</style>
