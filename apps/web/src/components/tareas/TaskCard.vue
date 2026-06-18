<script setup lang="ts">
import { computed } from 'vue';
import Avatar from '@/components/ui/Avatar.vue';
import Chip, { type ChipTone } from '@/components/ui/Chip.vue';
import type { TaskSummaryDto } from '@/stores/boards';

const props = defineProps<{
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
    class="atl-card atl-task-card flex flex-col text-left w-full cursor-grab select-none"
    :data-readable-id="task.readable_id"
    :style="{
      gap: '7px',
      padding: '9px 10px',
      backgroundColor: selected ? 'var(--c-selected, var(--c-raised))' : 'var(--c-raised)',
      border: `1px solid ${selected ? 'var(--c-primary)' : 'var(--c-border)'}`,
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
      v-if="task.priority || (task.labels && task.labels.length)"
      class="flex flex-wrap"
      style="gap: 5px;"
    >
      <Chip v-for="label in task.labels ?? []" :key="label" tone="info">{{ label }}</Chip>
      <Chip v-if="task.priority" :tone="priorityTone">{{ task.priority }}</Chip>
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
      <span v-if="task.assignees && task.assignees.length" class="flex items-center" style="gap: 3px;">
        <Avatar
          v-for="a in task.assignees"
          :key="`${a.type}:${a.id}`"
          :name="a.display_name ?? ''"
          :agent="a.type === 'api_key'"
          :size="18"
        />
      </span>
    </div>
  </button>
</template>

<style scoped>
.atl-task-card:active {
  cursor: grabbing;
}
</style>
