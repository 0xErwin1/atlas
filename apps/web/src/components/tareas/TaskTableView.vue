<script setup lang="ts">
/**
 * Task Table view: every board task as a sticky-header table, mirroring the
 * hi-fi Table design (taskboards.jsx). The seven columns are Task, Status,
 * Assignee, Priority, Est, Due, and Tags. Rows are the tasks flattened across
 * all columns. Clicking a row emits `select` so the host opens the detail pane;
 * the row matching `selectedReadableId` is highlighted.
 *
 * Data is real. The Due column reads the full task DTO's `due_date` (the bulk
 * summary omits it); the host fetches those details when this view activates, so
 * Due renders "—" while details load or when a task simply has no due date —
 * never a fabricated value. Status color is the user's registry choice for the
 * column value.
 */
import { computed } from 'vue';
import Avatar from '@/components/ui/Avatar.vue';
import Chip from '@/components/ui/Chip.vue';
import Icon from '@/components/ui/Icon.vue';
import { swatchById } from '@/lib/swatches';
import type { ColumnDto, TaskSummaryDto } from '@/stores/boards';
import { useBoardsStore } from '@/stores/boards';
import { useLabelColorsStore } from '@/stores/labelColors';

defineProps<{ ws: string; selectedReadableId: string | null }>();
const emit = defineEmits<{ select: [readableId: string]; open: [readableId: string] }>();

const boards = useBoardsStore();
const labelColors = useLabelColorsStore();

const PRIORITY_COLOR: Record<string, string> = {
  urgent: 'var(--c-danger)',
  high: 'var(--c-primary)',
  medium: 'var(--c-info)',
  low: 'var(--c-muted)',
};

interface StatusStyle {
  fg: string;
  bg: string;
}

interface Row {
  task: TaskSummaryDto;
  column: ColumnDto | undefined;
  status: StatusStyle;
  assignees: { id: string; name: string; agent: boolean }[];
  due: string;
}

const DEFAULT_STATUS: StatusStyle = { fg: 'var(--c-muted)', bg: 'rgba(179, 177, 173, 0.1)' };

// One swatch per column, resolved once per render instead of on every cell (the
// status pill read the swatch three times per row).
const statusByColumnId = computed<Map<string, StatusStyle>>(() => {
  const map = new Map<string, StatusStyle>();
  for (const column of boards.columns) {
    const swatch = swatchById(labelColors.colorFor(`status:${column.name}`));
    map.set(column.id, { fg: swatch.fg, bg: swatch.bg });
  }
  return map;
});

// Rows carry their derived status/assignees/due so the template reads plain
// fields instead of re-running (and re-allocating) per cell on every render.
const rows = computed<Row[]>(() =>
  boards.columns.flatMap((column) =>
    boards.filteredTasksByColumn(column.id).map((task) => ({
      task,
      column,
      status: statusByColumnId.value.get(column.id) ?? DEFAULT_STATUS,
      assignees: assigneesForTask(task),
      due: dueLabel(task.readable_id),
    })),
  ),
);

function priorityLabel(priority: string | null | undefined): string {
  if (priority === null || priority === undefined || priority === '') return '';
  return priority.charAt(0).toUpperCase() + priority.slice(1);
}

function assigneesForTask(task: TaskSummaryDto): { id: string; name: string; agent: boolean }[] {
  return (task.assignees ?? []).map((actor) => {
    const agent = actor.type === 'api_key';
    return {
      id: `${actor.type}:${actor.id}`,
      name: actor.display_name ?? (agent ? 'Agent' : 'User'),
      agent,
    };
  });
}

function dueLabel(readableId: string): string {
  const due = boards.taskDetail(readableId)?.due_date;
  if (due === null || due === undefined) return '—';
  const date = new Date(due);
  if (Number.isNaN(date.getTime())) return '—';
  return date.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
}
</script>

<template>
  <div class="atl-tt-scroll">
    <table class="atl-tt">
      <thead>
        <tr>
          <th class="atl-tt-icon" />
          <th class="atl-tt-th atl-tt-task">Task</th>
          <th class="atl-tt-th atl-tt-status">Status</th>
          <th class="atl-tt-th atl-tt-assignee">Assignee</th>
          <th class="atl-tt-th atl-tt-prio">Priority</th>
          <th class="atl-tt-th atl-tt-num">Est</th>
          <th class="atl-tt-th atl-tt-num">Due</th>
          <th class="atl-tt-th atl-tt-tags">Tags</th>
        </tr>
      </thead>
      <tbody>
        <tr
          v-for="row in rows"
          :key="row.task.id"
          class="atl-tt-row"
          :class="{ selected: row.task.readable_id === selectedReadableId }"
          @click="emit('select', row.task.readable_id)"
        >
          <td class="atl-tt-icon">
            <Icon name="square-kanban" :size="13" style="color: var(--c-primary);" />
          </td>
          <td class="atl-tt-task">
            <span class="atl-tt-title">{{ row.task.title }}</span>
          </td>
          <td class="atl-tt-status">
            <span
              class="atl-tt-statuspill"
              :style="{ color: row.status.fg, background: row.status.bg }"
            >
              <span class="atl-tt-statusdot" :style="{ background: row.status.fg }" />
              {{ row.column?.name ?? '—' }}
            </span>
          </td>
          <td class="atl-tt-assignee">
            <span v-if="row.assignees.length" class="atl-tt-avatars">
              <Avatar
                v-for="a in row.assignees"
                :key="a.id"
                :name="a.name"
                :agent="a.agent"
                :size="18"
              />
            </span>
            <span v-else class="atl-tt-noassignee" title="Unassigned">
              <Icon name="user" :size="11" />
            </span>
          </td>
          <td class="atl-tt-prio">
            <span v-if="row.task.priority" class="atl-tt-priocell">
              <Icon name="flag" :size="13" :style="{ color: PRIORITY_COLOR[row.task.priority] ?? 'var(--c-muted)' }" />
              {{ priorityLabel(row.task.priority) }}
            </span>
            <span v-else class="atl-tt-dash">—</span>
          </td>
          <td class="atl-tt-num">
            {{ row.task.estimate !== null && row.task.estimate !== undefined ? row.task.estimate : '—' }}
          </td>
          <td class="atl-tt-num">{{ row.due }}</td>
          <td class="atl-tt-tags">
            <span class="atl-tt-chips">
              <Chip
                v-for="label in row.task.labels ?? []"
                :key="label"
                :color="labelColors.colorFor(`tag:${label.toLowerCase()}`)"
              >
                {{ label }}
              </Chip>
            </span>
          </td>
        </tr>

        <tr v-if="rows.length === 0">
          <td class="atl-tt-empty" colspan="8">No tasks to show.</td>
        </tr>
      </tbody>
    </table>
  </div>
</template>

<style scoped>
.atl-tt-scroll {
  flex: 1 1 0;
  min-width: 0;
  min-height: 0;
  overflow: auto;
  background: var(--c-background);
}

.atl-tt {
  width: 100%;
  border-collapse: collapse;
  table-layout: fixed;
}

.atl-tt-th {
  height: 34px;
  padding: 0 0 0 0;
  font-size: var(--fs-xs);
  font-weight: var(--fw-semibold);
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--c-muted);
  text-align: left;
}

.atl-tt thead th {
  position: sticky;
  top: 0;
  z-index: 2;
  background: var(--c-panel);
  border-bottom: 1px solid var(--c-border);
}

.atl-tt thead tr th:first-child {
  padding-left: 14px;
}

.atl-tt thead tr th:last-child {
  padding-right: 14px;
}

.atl-tt-icon {
  width: 36px;
}

.atl-tt-task {
  width: auto;
}

.atl-tt-status {
  width: 132px;
}

.atl-tt-assignee {
  width: 140px;
}

.atl-tt-prio {
  width: 110px;
}

.atl-tt-num {
  width: 72px;
  text-align: right;
  padding-right: 8px;
  font-family: var(--font-mono);
  font-size: var(--fs-sm);
  color: var(--c-muted);
}

.atl-tt-tags {
  width: 150px;
}

.atl-tt-row {
  height: 38px;
  cursor: pointer;
}

.atl-tt-row td {
  border-bottom: 1px solid var(--c-border);
  vertical-align: middle;
}

.atl-tt-row td:first-child {
  padding-left: 14px;
}

.atl-tt-row td:last-child {
  padding-right: 14px;
}

.atl-tt-row:hover {
  background: var(--c-raised);
}

.atl-tt-row.selected {
  background: var(--c-selection);
  box-shadow: inset 2px 0 0 var(--c-primary);
}

.atl-tt-title {
  display: block;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: var(--fs-lg);
  color: var(--c-foreground);
}

.atl-tt-statuspill {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 20px;
  padding: 0 8px 0 7px;
  border-radius: 3px;
  font-size: var(--fs-xs);
  font-weight: var(--fw-semibold);
  white-space: nowrap;
}

.atl-tt-statusdot {
  width: 6px;
  height: 6px;
  border-radius: var(--r-full);
  flex: 0 0 auto;
}

.atl-tt-avatars {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.atl-tt-noassignee {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  border: 1px dashed var(--c-muted);
  border-radius: 2px;
  color: var(--c-muted);
}

.atl-tt-priocell {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: var(--fs-sm);
  color: var(--c-foreground);
}

.atl-tt-dash {
  color: var(--c-muted);
}

.atl-tt-chips {
  display: flex;
  gap: 5px;
  flex-wrap: wrap;
}

.atl-tt-empty {
  padding: 12px 14px;
  font-size: var(--fs-sm);
  color: var(--c-muted);
}
</style>
