<script setup lang="ts">
import { computed } from 'vue';
import { VueDraggable } from 'vue-draggable-plus';
import TaskCard from '@/components/tareas/TaskCard.vue';
import Icon from '@/components/ui/Icon.vue';
import { resolveDropTarget } from '@/composables/kanbanDrop';
import { useInlineEdit } from '@/composables/useInlineEdit';
import type { ColumnDto, TaskSummaryDto } from '@/stores/boards';

const props = defineProps<{
  column: ColumnDto;
  tasks: TaskSummaryDto[];
}>();

const emit = defineEmits<{
  /** A drop landed in this column: (readableId, columnId, toIndex). */
  drop: [readableId: string, columnId: string, toIndex: number];
  /** Quick-add: create a task in this column with the given title. */
  create: [columnId: string, title: string];
  open: [readableId: string];
}>();

const {
  active: adding,
  value: addValue,
  inputRef,
  start: startAdd,
  commit: commitAdd,
  onKeydown: onAddKeydown,
} = useInlineEdit<'task'>((title) => emit('create', props.column.id, title));

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
      <span class="flex-1" />
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
        @open="(id) => emit('open', id)"
      />
    </VueDraggable>
  </div>
</template>

<style scoped>
.atl-card-ghost {
  opacity: 0.4;
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
