<script setup lang="ts">
/**
 * A single row of the task List view — the leading status marker, title + label
 * chips, readable id, assignee, priority, status and estimate laid out on the
 * shared 7-column grid. Purely presentational: every option list, open state and
 * derived flag is passed in, and all interactions are emitted up so the parent
 * (TaskListView) keeps one source of truth (one open picker at a time, the move /
 * assign / priority mutations, the context menu).
 *
 * The same row renders a top-level task and its nested sub-tasks: `indent` shifts
 * the name column, and `expandable`/`expanded` drive the tree disclosure caret so
 * sub-tasks are reachable from the list without opening the parent.
 */

import AssigneeAvatars from '@/components/tareas/AssigneeAvatars.vue';
import TaskRowPicker, { type PickerOption } from '@/components/tareas/TaskRowPicker.vue';
import Chip from '@/components/ui/Chip.vue';
import Icon from '@/components/ui/Icon.vue';
import { PRIORITY_COLOR, priorityLabel } from '@/lib/taskPriority';
import type { TaskSummaryDto } from '@/stores/boards';
import { useLabelColorsStore } from '@/stores/labelColors';

const props = withDefaults(
  defineProps<{
    task: TaskSummaryDto;
    selected: boolean;
    done: boolean;
    ringColor: string;
    statusName: string;
    statusOptions: PickerOption[];
    assigneeOptions: PickerOption[];
    priorityOptions: PickerOption[];
    statusOpen: boolean;
    assigneeOpen: boolean;
    priorityOpen: boolean;
    /** Nesting depth: 0 for a board task, 1+ for sub-tasks. */
    indent?: number;
    /** The task has sub-tasks, so it shows a disclosure caret. */
    expandable?: boolean;
    expanded?: boolean;
  }>(),
  { indent: 0, expandable: false, expanded: false },
);

const emit = defineEmits<{
  select: [readableId: string];
  menu: [task: TaskSummaryDto, event: MouseEvent];
  copy: [task: TaskSummaryDto];
  'toggle-expand': [task: TaskSummaryDto];
  'status-open': [task: TaskSummaryDto, value: boolean];
  'assignee-open': [task: TaskSummaryDto, value: boolean];
  'priority-open': [task: TaskSummaryDto, value: boolean];
  'status-pick': [task: TaskSummaryDto, value: string];
  'assignee-pick': [task: TaskSummaryDto, value: string];
  'priority-pick': [task: TaskSummaryDto, value: string];
}>();

const labelColors = useLabelColorsStore();
</script>

<template>
  <button
    type="button"
    class="atl-tl-row"
    :class="{ selected }"
    :data-readable-id="task.readable_id"
    @click="emit('select', task.readable_id)"
    @contextmenu.prevent="emit('menu', task, $event)"
  >
    <TaskRowPicker
      class="atl-tl-pick"
      :options="statusOptions"
      :open="statusOpen"
      @update:open="(v: boolean) => emit('status-open', task, v)"
      @pick="(v: string) => emit('status-pick', task, v)"
    >
      <template #trigger>
        <span v-if="done" class="atl-tl-marker done" title="Change status">
          <Icon name="check" :size="10" :stroke-width="2.6" />
        </span>
        <span v-else class="atl-tl-marker" title="Change status" :style="{ borderColor: ringColor }" />
      </template>
    </TaskRowPicker>

    <span class="atl-tl-name" :style="{ paddingLeft: `${indent * 18}px` }">
      <span
        v-if="expandable"
        class="atl-tl-expand"
        :class="{ open: expanded }"
        role="button"
        :aria-label="expanded ? 'Collapse sub-tasks' : 'Expand sub-tasks'"
        @click.stop="emit('toggle-expand', task)"
      >
        <Icon name="chevron-right" :size="13" />
      </span>
      <span v-else-if="indent > 0" class="atl-tl-expand-spacer" />

      <span class="atl-tl-title" :class="{ muted: done }">{{ task.title }}</span>
      <span v-if="(task.labels ?? []).length > 0" class="atl-tl-labels">
        <Chip
          v-for="label in task.labels ?? []"
          :key="label"
          truncate
          :color="labelColors.colorFor(`tag:${label.toLowerCase()}`)"
        >
          {{ label }}
        </Chip>
      </span>
    </span>

    <span class="atl-tl-id">
      <span class="atl-tl-id-text">{{ task.readable_id }}</span>
      <button
        type="button"
        class="atl-tl-copy"
        :title="`Copy ${task.readable_id}`"
        @click.stop="emit('copy', task)"
      >
        <Icon name="copy" :size="12" />
      </button>
    </span>

    <TaskRowPicker
      class="atl-tl-pick atl-tl-assignee"
      :options="assigneeOptions"
      width="220px"
      :open="assigneeOpen"
      @update:open="(v: boolean) => emit('assignee-open', task, v)"
      @pick="(v: string) => emit('assignee-pick', task, v)"
    >
      <template #trigger>
        <AssigneeAvatars
          v-if="task.assignees && task.assignees.length"
          :assignees="task.assignees"
          :max="3"
          :size="18"
        />
        <span v-else class="atl-tl-noassignee" title="Assign">
          <Icon name="user" :size="11" />
        </span>
      </template>
    </TaskRowPicker>

    <TaskRowPicker
      class="atl-tl-pick atl-tl-prio"
      :options="priorityOptions"
      :open="priorityOpen"
      @update:open="(v: boolean) => emit('priority-open', task, v)"
      @pick="(v: string) => emit('priority-pick', task, v)"
    >
      <template #trigger>
        <span class="atl-tl-prio-inner" title="Change priority">
          <template v-if="task.priority">
            <Icon
              name="flag"
              :size="13"
              :style="{ color: PRIORITY_COLOR[task.priority] ?? 'var(--c-muted)' }"
            />
            {{ priorityLabel(task.priority) }}
          </template>
          <span v-else class="atl-tl-prio-empty">—</span>
        </span>
      </template>
    </TaskRowPicker>

    <span class="atl-tl-status">{{ statusName }}</span>

    <span class="atl-tl-est">{{ task.estimate !== null && task.estimate !== undefined ? `${task.estimate} pts` : '—' }}</span>
  </button>
</template>

<style scoped>
.atl-tl-row {
  display: grid;
  grid-template-columns: 15px minmax(0, 1fr) 84px 64px 96px 110px 64px;
  align-items: center;
  column-gap: 10px;
  width: 100%;
  height: 38px;
  padding: 0 12px 0 10px;
  border: none;
  border-radius: 3px;
  background: transparent;
  text-align: left;
  cursor: pointer;
  content-visibility: auto;
  contain-intrinsic-size: auto 38px;
}

.atl-tl-row:hover {
  background: var(--c-raised);
}

.atl-tl-row.selected {
  background: var(--c-selection);
  box-shadow: inset 2px 0 0 var(--c-primary);
}

.atl-tl-pick {
  display: inline-flex;
  align-items: center;
  min-width: 0;
}

.atl-tl-marker {
  width: 15px;
  height: 15px;
  border-radius: var(--r-full);
  border: 1.6px solid var(--c-muted);
  flex: 0 0 auto;
  transition: transform 0.1s ease;
}

.atl-tl-marker:hover {
  transform: scale(1.12);
}

.atl-tl-marker.done {
  display: flex;
  align-items: center;
  justify-content: center;
  border: none;
  background: var(--c-success);
  color: var(--c-background);
}

.atl-tl-name {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  /* Clip only at the cell boundary in the pathological many-labels case, so labels
     never bleed into the next column. */
  overflow: hidden;
}

.atl-tl-expand {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  flex: 0 0 auto;
  color: var(--c-muted);
  border-radius: 3px;
  cursor: pointer;
  transition: transform 0.12s ease, color 0.12s ease;
}

.atl-tl-expand:hover {
  color: var(--c-foreground);
  background: var(--c-raised);
}

.atl-tl-expand.open {
  transform: rotate(90deg);
}

.atl-tl-expand-spacer {
  width: 16px;
  flex: 0 0 auto;
}

.atl-tl-title {
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

.atl-tl-labels {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  /* Never shrink: a long title ellipsizes (the title alone absorbs the row's
     slack) while the label chips keep their intrinsic width instead of clipping. */
  flex: 0 0 auto;
}

.atl-tl-prio {
  min-width: 0;
}

.atl-tl-prio-inner {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  height: 24px;
  padding: 0 6px;
  border-radius: var(--r-sm);
  font-size: var(--fs-sm);
  color: var(--c-foreground);
}

.atl-tl-prio-inner:hover {
  background: var(--c-raised);
}

.atl-tl-prio-empty {
  color: var(--c-muted);
}

.atl-tl-status {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  text-align: center;
  font-size: var(--fs-sm);
  color: var(--c-muted);
}

.atl-tl-est {
  text-align: right;
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--c-muted);
}

.atl-tl-assignee {
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

.atl-tl-assignee :deep(.atl-rp-trigger) {
  border-radius: var(--r-full);
}

.atl-tl-assignee :deep(.atl-rp-trigger:hover) {
  background: var(--c-raised);
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
  position: relative;
  display: inline-flex;
  align-items: center;
  justify-content: flex-end;
  min-width: 0;
}

.atl-tl-id-text {
  font-family: var(--font-mono);
  font-size: var(--fs-xs);
  color: var(--c-muted);
  white-space: nowrap;
}

.atl-tl-copy {
  position: absolute;
  right: -2px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  padding: 0;
  border: none;
  border-radius: 3px;
  background: var(--c-raised);
  color: var(--c-muted);
  cursor: pointer;
  opacity: 0;
  transition: opacity 0.12s ease;
}

.atl-tl-copy:hover {
  color: var(--c-foreground);
}

.atl-tl-row:hover .atl-tl-copy {
  opacity: 1;
}
</style>
