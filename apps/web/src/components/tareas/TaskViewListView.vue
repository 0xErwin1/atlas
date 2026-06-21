<script setup lang="ts">
/**
 * Flat task list for cross-board views (predefined and custom). Unlike
 * TaskListView, there is no column grouping and no dependency on the boards
 * store: the status text and board name travel on each TaskSummaryDto
 * (`column_name` / `board_name`), so a single feed can mix tasks from any
 * board. The row visual language is shared verbatim with TaskListView via the
 * `atl-tl-*` classes so the two views feel identical.
 *
 * Clicking a row emits `select` (host opens the detail pane); double-clicking
 * emits `open`. The status color is the user's registry choice for the column
 * value, never inferred from text.
 */
import { computed } from 'vue';
import Avatar from '@/components/ui/Avatar.vue';
import Chip from '@/components/ui/Chip.vue';
import Icon from '@/components/ui/Icon.vue';
import { relativeTime } from '@/lib/relativeTime';
import { swatchById } from '@/lib/swatches';
import type { TaskSummaryDto } from '@/stores/boards';
import { useLabelColorsStore } from '@/stores/labelColors';

const props = defineProps<{
  tasks: TaskSummaryDto[];
  selectedReadableId?: string | null;
}>();

const emit = defineEmits<{
  select: [readableId: string];
  open: [readableId: string];
}>();

const labelColors = useLabelColorsStore();

const PRIORITY_COLOR: Record<string, string> = {
  urgent: 'var(--c-danger)',
  high: 'var(--c-primary)',
  medium: 'var(--c-info)',
  low: 'var(--c-muted)',
};

function statusColor(columnName: string): string {
  return swatchById(labelColors.colorFor(`status:${columnName}`)).fg;
}

function isDone(task: TaskSummaryDto): boolean {
  return task.column_name.trim().toLowerCase() === 'done';
}

function priorityLabel(priority: string | null | undefined): string {
  if (priority === null || priority === undefined || priority === '') return 'No priority';
  return priority.charAt(0).toUpperCase() + priority.slice(1);
}

function assigneeForTask(task: TaskSummaryDto): { name: string; agent: boolean } | null {
  const actor = task.assignees?.[0];
  if (actor === undefined) return null;
  const agent = actor.type === 'api_key';
  return { name: actor.display_name ?? (agent ? 'Agent' : 'User'), agent };
}

const isEmpty = computed(() => props.tasks.length === 0);
</script>

<template>
  <div class="atl-tl-scroll">
    <div class="atl-tl-inner">
      <button
        v-for="task in tasks"
        :key="task.id"
        type="button"
        class="atl-tl-row"
        :class="{ selected: task.readable_id === selectedReadableId }"
        @click="emit('select', task.readable_id)"
        @dblclick="emit('open', task.readable_id)"
      >
        <span
          v-if="isDone(task)"
          class="atl-tl-marker done"
        >
          <Icon name="check" :size="10" :stroke-width="2.6" />
        </span>
        <span
          v-else
          class="atl-tl-marker"
          :style="{ borderColor: statusColor(task.column_name) }"
        />

        <span
          class="atl-tl-status"
          :style="{ color: statusColor(task.column_name) }"
        >
          {{ task.column_name }}
        </span>

        <span class="atl-tl-board">{{ task.board_name }}</span>

        <span class="atl-tl-title" :class="{ muted: isDone(task) }">{{ task.title }}</span>

        <span class="atl-tl-trailing">
          <Chip
            v-for="label in task.labels ?? []"
            :key="label"
            :color="labelColors.colorFor(`tag:${label.toLowerCase()}`)"
          >
            {{ label }}
          </Chip>

          <span class="atl-tl-prio">
            <template v-if="task.priority">
              <Icon
                name="flag"
                :size="13"
                :style="{ color: PRIORITY_COLOR[task.priority] ?? 'var(--c-muted)' }"
              />
              {{ priorityLabel(task.priority) }}
            </template>
          </span>

          <span class="atl-tl-est">{{ task.estimate !== null && task.estimate !== undefined ? `${task.estimate} pts` : '—' }}</span>

          <span class="atl-tl-assignee">
            <template v-if="assigneeForTask(task)">
              <Avatar :name="assigneeForTask(task)!.name" :agent="assigneeForTask(task)!.agent" :size="18" />
            </template>
            <span v-else class="atl-tl-noassignee" title="Unassigned">
              <Icon name="user" :size="11" />
            </span>
          </span>

          <span class="atl-tl-id">{{ task.readable_id }}</span>
          <span class="atl-tl-updated">{{ relativeTime(task.updated_at) }}</span>
        </span>
      </button>

      <p v-if="isEmpty" class="atl-tl-empty">No tasks to show.</p>
    </div>
  </div>
</template>

<style scoped>
.atl-tl-scroll {
  flex: 1 1 0;
  min-width: 0;
  min-height: 0;
  overflow: auto;
  padding: 14px 16px;
  background: var(--c-background);
}

.atl-tl-inner {
  max-width: 1100px;
}

.atl-tl-row {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  height: 38px;
  padding: 0 12px 0 10px;
  border: none;
  border-radius: 3px;
  background: transparent;
  text-align: left;
  cursor: pointer;
}

.atl-tl-row:hover {
  background: var(--c-raised);
}

.atl-tl-row.selected {
  background: var(--c-selection);
  box-shadow: inset 2px 0 0 var(--c-primary);
}

.atl-tl-marker {
  width: 15px;
  height: 15px;
  border-radius: var(--r-full);
  border: 1.6px solid var(--c-muted);
  flex: 0 0 auto;
}

.atl-tl-marker.done {
  display: flex;
  align-items: center;
  justify-content: center;
  border: none;
  background: var(--c-success);
  color: var(--c-background);
}

.atl-tl-status {
  flex: 0 0 auto;
  width: 96px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-sm);
  font-weight: var(--fw-semibold);
}

.atl-tl-board {
  flex: 0 0 auto;
  max-width: 130px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  padding: 2px 7px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-sm);
  background: var(--c-raised);
  font-size: var(--fs-xs);
  color: var(--c-muted);
}

.atl-tl-title {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-lg);
  color: var(--c-foreground);
}

.atl-tl-title.muted {
  color: var(--c-muted);
}

.atl-tl-trailing {
  display: flex;
  align-items: center;
  gap: 9px;
  flex: 0 0 auto;
}

.atl-tl-prio {
  display: inline-flex;
  align-items: center;
  justify-content: flex-end;
  gap: 6px;
  width: 86px;
  font-size: var(--fs-sm);
  color: var(--c-foreground);
}

.atl-tl-est {
  width: 48px;
  text-align: right;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--c-muted);
}

.atl-tl-assignee {
  display: inline-flex;
  align-items: center;
  justify-content: flex-end;
  width: 22px;
}

.atl-tl-noassignee {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  border: 1px dashed var(--c-muted);
  border-radius: 2px;
  color: var(--c-muted);
  flex: 0 0 auto;
}

.atl-tl-id {
  width: 52px;
  text-align: right;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--c-muted);
}

.atl-tl-updated {
  width: 64px;
  text-align: right;
  font-size: var(--fs-xs);
  color: var(--c-muted);
}

.atl-tl-empty {
  padding: 8px;
  font-size: var(--fs-sm);
  color: var(--c-muted);
}
</style>
