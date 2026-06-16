<script setup lang="ts">
import { computed } from 'vue';
import Chip, { type ChipTone } from '@/components/ui/Chip.vue';
import type { TaskSummaryDto } from '@/stores/boards';

const props = defineProps<{
  task: TaskSummaryDto;
}>();

defineEmits<{
  open: [readableId: string];
}>();

const PRIORITY_TONE: Record<string, ChipTone> = {
  urgent: 'danger',
  high: 'warning',
  medium: 'info',
  low: 'neutral',
};

const priorityTone = computed<ChipTone>(() => {
  const p = props.task.priority?.toLowerCase() ?? '';
  return PRIORITY_TONE[p] ?? 'neutral';
});
</script>

<template>
  <button
    type="button"
    class="atl-task-card flex flex-col text-left w-full cursor-grab select-none"
    :data-readable-id="task.readable_id"
    style="
      gap: 7px;
      padding: 9px 10px;
      background-color: var(--c-raised);
      border: 1px solid var(--c-border);
      border-radius: var(--r-md);
    "
    @click="$emit('open', task.readable_id)"
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
      v-if="task.priority"
      class="flex items-center gap-1 flex-wrap"
    >
      <Chip :tone="priorityTone">{{ task.priority }}</Chip>
    </div>

    <div class="flex items-center gap-2">
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
    </div>
  </button>
</template>

<style scoped>
.atl-task-card:active {
  cursor: grabbing;
}

.atl-task-card:hover {
  border-color: var(--c-selection);
}
</style>
