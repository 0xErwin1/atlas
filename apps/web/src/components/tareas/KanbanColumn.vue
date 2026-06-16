<script setup lang="ts">
import { computed } from 'vue';
import { VueDraggable } from 'vue-draggable-plus';
import TaskCard from '@/components/tareas/TaskCard.vue';
import { resolveDropTarget } from '@/composables/kanbanDrop';
import type { ColumnDto, TaskSummaryDto } from '@/stores/boards';

const props = defineProps<{
  column: ColumnDto;
  tasks: TaskSummaryDto[];
}>();

const emit = defineEmits<{
  /** A drop landed in this column: (readableId, columnId, toIndex). */
  drop: [readableId: string, columnId: string, toIndex: number];
  open: [readableId: string];
}>();

const DOT_COLOR: Record<string, string> = {
  backlog: 'var(--c-muted)',
  'in progress': 'var(--c-info)',
  inprogress: 'var(--c-info)',
  review: 'var(--c-primary)',
  done: 'var(--c-success)',
};

const dotColor = computed(() => DOT_COLOR[props.column.name.toLowerCase()] ?? 'var(--c-muted)');

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

function onChange(event: unknown): void {
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
    style="width: 250px; flex: 0 0 250px;"
  >
    <div
      class="flex items-center"
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
        style="font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--c-foreground);"
      >
        {{ column.name }}
      </span>
      <span
        style="font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--c-muted);"
      >
        {{ tasks.length }}
      </span>
    </div>

    <VueDraggable
      v-model="model"
      :group="'kanban'"
      :animation="150"
      item-key="id"
      class="flex flex-col"
      style="gap: 8px; min-height: 24px;"
      ghost-class="atl-card-ghost"
      @change="onChange"
    >
      <TaskCard
        v-for="task in tasks"
        :key="task.id"
        :task="task"
        @open="(id) => emit('open', id)"
      />
    </VueDraggable>
  </div>
</template>

<style scoped>
.atl-card-ghost {
  opacity: 0.4;
}
</style>
