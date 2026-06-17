<script setup lang="ts">
import KanbanColumn from '@/components/tareas/KanbanColumn.vue';
import { useKanbanMove } from '@/composables/useKanbanMove';
import { useBoardsStore } from '@/stores/boards';
import { useUiStore } from '@/stores/ui';

const props = defineProps<{
  ws: string;
}>();

const emit = defineEmits<{
  open: [readableId: string];
}>();

const boards = useBoardsStore();
const ui = useUiStore();
const { move } = useKanbanMove(props.ws);

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
</script>

<template>
  <div
    class="flex flex-1 overflow-x-auto"
    style="gap: 14px; padding: 16px; background-color: var(--c-background);"
  >
    <KanbanColumn
      v-for="column in boards.columns"
      :key="column.id"
      :column="column"
      :tasks="boards.tasksByColumn(column.id)"
      @drop="onDrop"
      @create="onCreate"
      @open="(id) => emit('open', id)"
    />

    <p
      v-if="boards.columns.length === 0 && !boards.loading"
      style="font-size: var(--fs-sm); color: var(--c-muted); padding: 8px;"
    >
      This board has no columns yet.
    </p>
  </div>
</template>
