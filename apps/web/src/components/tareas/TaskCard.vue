<script setup lang="ts">
import AssigneeAvatars from '@/components/tareas/AssigneeAvatars.vue';
import Chip from '@/components/ui/Chip.vue';
import type { TaskSummaryDto } from '@/stores/boards';
import { useLabelColorsStore } from '@/stores/labelColors';

const labelColors = useLabelColorsStore();

defineProps<{
  task: TaskSummaryDto;
  selected?: boolean;
}>();

defineEmits<{
  /** Single click: select the card (peek in the inspector). */
  select: [readableId: string];
  /** Double click: open the full task detail. */
  open: [readableId: string];
  menu: [readableId: string, event: MouseEvent];
}>();
</script>

<template>
  <button
    type="button"
    class="atl-card atl-task-card flex flex-col text-left w-full cursor-grab select-none"
    :data-readable-id="task.readable_id"
    :style="{
      gap: '7px',
      padding: '9px 10px',
      backgroundColor: selected ? 'var(--c-selected, var(--c-raised))' : 'var(--c-raised)',
      border: `1px solid ${selected ? 'var(--c-primary)' : 'var(--c-border)'}`,
      boxShadow: selected ? 'inset 0 0 0 1px var(--c-primary)' : 'none',
      borderRadius: 'var(--r-md)',
    }"
    @click="$emit('select', task.readable_id)"
    @dblclick="$emit('open', task.readable_id)"
    @contextmenu.prevent="$emit('menu', task.readable_id, $event)"
  >
    <span
      style="
        font-size: 12.5px;
        font-weight: var(--fw-medium);
        line-height: 1.35;
        color: var(--c-foreground);
      "
    >
      {{ task.title }}
    </span>

    <div
      v-if="task.labels && task.labels.length"
      class="flex flex-wrap"
      style="gap: 5px;"
    >
      <Chip v-for="label in task.labels ?? []" :key="label" :color="labelColors.colorFor(`tag:${label.toLowerCase()}`)">{{ label }}</Chip>
    </div>

    <div class="flex items-center" style="gap: 8px;">
      <span
        style="
          font-family: var(--font-mono);
          font-size: var(--fs-xs);
          color: var(--c-muted);
        "
      >
        {{ task.readable_id }}
      </span>
      <span class="flex-1" />
      <AssigneeAvatars
        v-if="task.assignees && task.assignees.length"
        :assignees="task.assignees"
        :max="3"
        :size="18"
      />
    </div>
  </button>
</template>

<style scoped>
.atl-task-card:active {
  cursor: grabbing;
}
</style>
