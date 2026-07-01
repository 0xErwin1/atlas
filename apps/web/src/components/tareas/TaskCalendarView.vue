<script setup lang="ts">
/**
 * Task Calendar view: the board's tasks laid out on a month grid by their real
 * due date, mirroring the hi-fi Calendar design (taskboards.jsx). The visible
 * month defaults to the current month and can be paged. Tasks with no due date
 * are listed in an "Unscheduled" strip rather than placed anywhere — the data
 * model has a due date but no other scheduling field, so nothing is invented.
 *
 * Due dates come from the full task DTOs (the bulk summary omits them); the host
 * fetches those when this view activates. Status color is the user's registry
 * choice for the task's column value.
 */
import { computed, ref } from 'vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import ContextMenu from '@/components/ui/ContextMenu.vue';
import Icon from '@/components/ui/Icon.vue';
import PromptDialog from '@/components/ui/PromptDialog.vue';
import { useContextMenu } from '@/composables/useContextMenu';
import { useTaskInteractions } from '@/composables/useTaskInteractions';
import { type Swatch, swatchById } from '@/lib/swatches';
import type { TaskSummaryDto } from '@/stores/boards';
import { useBoardsStore } from '@/stores/boards';
import { useLabelColorsStore } from '@/stores/labelColors';
import type { TaskViewMode } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

const props = defineProps<{ ws: string; selectedReadableId: string | null }>();
const emit = defineEmits<{ select: [readableId: string, mode?: TaskViewMode]; open: [readableId: string] }>();

const boards = useBoardsStore();
const labelColors = useLabelColorsStore();
const workspace = useWorkspaceStore();
const ti = useTaskInteractions(props.ws);
const menu = useContextMenu();

async function onMenu(readableId: string, event: MouseEvent): Promise<void> {
  ti.menuReadableId.value = readableId;
  menu.openAt(event);

  void workspace.loadMembers(props.ws);
  await Promise.all(workspace.projects.map((p) => boards.loadBoardsForProject(props.ws, p.slug)));
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

const WEEKDAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
const MONTH_NAMES = [
  'January',
  'February',
  'March',
  'April',
  'May',
  'June',
  'July',
  'August',
  'September',
  'October',
  'November',
  'December',
];

const today = new Date();
const viewYear = ref(today.getFullYear());
const viewMonth = ref(today.getMonth());

const monthLabel = computed(() => `${MONTH_NAMES[viewMonth.value]} ${viewYear.value}`);

function shiftMonth(delta: number): void {
  const next = new Date(viewYear.value, viewMonth.value + delta, 1);
  viewYear.value = next.getFullYear();
  viewMonth.value = next.getMonth();
}

function jumpToToday(): void {
  viewYear.value = today.getFullYear();
  viewMonth.value = today.getMonth();
}

const allTasks = computed<TaskSummaryDto[]>(() =>
  boards.columns.flatMap((c) => boards.filteredTasksByColumn(c.id)),
);

function dueDate(readableId: string): Date | null {
  const raw = boards.taskDetail(readableId)?.due_date;
  if (raw === null || raw === undefined) return null;
  const date = new Date(raw);
  return Number.isNaN(date.getTime()) ? null : date;
}

// Resolve one swatch per column once per render instead of a linear
// `columns.find` on every chip (each chip reads it twice: background + dot).
const swatchByColumnId = computed<Map<string, Swatch>>(() => {
  const map = new Map<string, Swatch>();
  for (const column of boards.columns) {
    map.set(column.id, swatchById(labelColors.colorFor(`status:${column.name}`)));
  }
  return map;
});

function statusSwatch(task: TaskSummaryDto): Swatch {
  return swatchByColumnId.value.get(task.column_id) ?? swatchById(labelColors.colorFor('status:'));
}

interface Cell {
  key: number;
  day: number;
  inMonth: boolean;
  isToday: boolean;
  weekend: boolean;
  tasks: TaskSummaryDto[];
}

// Resolve every task's due date once per render (each `dueDate` call does a store
// lookup and allocates a Date), then bucket the current month's tasks by day so
// each cell is an O(1) lookup instead of re-filtering every task (O(cells×tasks)).
const taskDueDates = computed<{ task: TaskSummaryDto; due: Date | null }[]>(() =>
  allTasks.value.map((task) => ({ task, due: dueDate(task.readable_id) })),
);

const tasksByDay = computed<Map<number, TaskSummaryDto[]>>(() => {
  const map = new Map<number, TaskSummaryDto[]>();
  for (const { task, due } of taskDueDates.value) {
    if (due === null) continue;
    if (due.getFullYear() !== viewYear.value || due.getMonth() !== viewMonth.value) continue;

    const day = due.getDate();
    const list = map.get(day);
    if (list) list.push(task);
    else map.set(day, [task]);
  }
  return map;
});

const cells = computed<Cell[]>(() => {
  const first = new Date(viewYear.value, viewMonth.value, 1);
  const mondayOffset = (first.getDay() + 6) % 7;
  const daysInMonth = new Date(viewYear.value, viewMonth.value + 1, 0).getDate();
  const total = Math.ceil((mondayOffset + daysInMonth) / 7) * 7;

  const result: Cell[] = [];
  for (let i = 0; i < total; i += 1) {
    const cellDate = new Date(viewYear.value, viewMonth.value, i - mondayOffset + 1);
    const inMonth = cellDate.getMonth() === viewMonth.value;
    const weekday = i % 7;

    result.push({
      key: i,
      day: cellDate.getDate(),
      inMonth,
      isToday:
        cellDate.getFullYear() === today.getFullYear() &&
        cellDate.getMonth() === today.getMonth() &&
        cellDate.getDate() === today.getDate(),
      weekend: weekday >= 5,
      tasks: inMonth ? (tasksByDay.value.get(cellDate.getDate()) ?? []) : [],
    });
  }
  return result;
});

const weeks = computed<Cell[][]>(() => {
  const rows: Cell[][] = [];
  for (let i = 0; i < cells.value.length; i += 7) rows.push(cells.value.slice(i, i + 7));
  return rows;
});

const unscheduled = computed<TaskSummaryDto[]>(() =>
  taskDueDates.value.filter((d) => d.due === null).map((d) => d.task),
);
</script>

<template>
  <div class="atl-cal">
    <div class="atl-cal-head">
      <span class="atl-cal-month">{{ monthLabel }}</span>
      <div class="flex" style="gap: 2px;">
        <button type="button" class="atl-gbtn" title="Previous month" style="width: 24px; height: 24px;" @click="shiftMonth(-1)">
          <Icon name="chevron-left" :size="15" />
        </button>
        <button type="button" class="atl-gbtn" title="Next month" style="width: 24px; height: 24px;" @click="shiftMonth(1)">
          <Icon name="chevron-right" :size="15" />
        </button>
      </div>
      <button type="button" class="atl-gbtn" title="Jump to today" style="height: 24px;" @click="jumpToToday">Today</button>
      <div style="flex: 1;" />
      <span v-if="boards.detailsLoading" class="atl-cal-note">Loading due dates…</span>
      <span v-else class="atl-cal-note">Showing by due date</span>
    </div>

    <div class="atl-cal-weekdays">
      <div
        v-for="(w, i) in WEEKDAYS"
        :key="w"
        class="atl-cal-weekday"
        :class="{ weekend: i >= 5, last: i === 6 }"
      >
        {{ w }}
      </div>
    </div>

    <div class="atl-cal-grid">
      <div v-for="(week, wi) in weeks" :key="wi" class="atl-cal-week" :class="{ last: wi === weeks.length - 1 }">
        <div
          v-for="(cell, di) in week"
          :key="cell.key"
          class="atl-cal-cell"
          :class="{ today: cell.isToday, weekend: cell.weekend && cell.inMonth, last: di === 6 }"
        >
          <div class="atl-cal-daynum-row">
            <span class="atl-cal-daynum" :class="{ today: cell.isToday, out: !cell.inMonth }">{{ cell.day }}</span>
          </div>
          <button
            v-for="t in cell.tasks"
            :key="t.id"
            type="button"
            class="atl-cal-chip"
            :class="{ selected: t.readable_id === selectedReadableId }"
            :style="{ background: statusSwatch(t).bg }"
            :title="t.title"
            @click="emit('select', t.readable_id)"
            @contextmenu.prevent="onMenu(t.readable_id, $event)"
          >
            <span class="atl-cal-chipdot" :style="{ background: statusSwatch(t).fg }" />
            <span class="atl-cal-chiptitle">{{ t.title }}</span>
          </button>
        </div>
      </div>
    </div>

    <div v-if="unscheduled.length > 0" class="atl-cal-unsched">
      <span class="atl-cal-unsched-label">Unscheduled · {{ unscheduled.length }}</span>
      <button
        v-for="t in unscheduled"
        :key="t.id"
        type="button"
        class="atl-cal-chip"
        :class="{ selected: t.readable_id === selectedReadableId }"
        :style="{ background: statusSwatch(t).bg }"
        :title="t.title"
        @click="emit('select', t.readable_id)"
        @contextmenu.prevent="onMenu(t.readable_id, $event)"
      >
        <span class="atl-cal-chipdot" :style="{ background: statusSwatch(t).fg }" />
        <span class="atl-cal-chiptitle">{{ t.title }}</span>
      </button>
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
.atl-cal {
  flex: 1 1 0;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  background: var(--c-background);
}

.atl-cal-head {
  display: flex;
  align-items: center;
  gap: 10px;
  height: 46px;
  flex: 0 0 46px;
  padding: 0 16px;
  border-bottom: 1px solid var(--c-border);
}

.atl-cal-month {
  font-size: 15px;
  font-weight: var(--fw-bold);
  color: var(--c-foreground);
}

.atl-cal-note {
  font-size: 11.5px;
  color: var(--c-muted);
}

.atl-cal-weekdays {
  display: flex;
  flex: 0 0 28px;
  border-bottom: 1px solid var(--c-border);
}

.atl-cal-weekday {
  flex: 1;
  display: flex;
  align-items: center;
  padding: 0 8px;
  font-size: 10.5px;
  font-weight: var(--fw-semibold);
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--c-muted);
  border-right: 1px solid var(--c-border);
}

.atl-cal-weekday.weekend {
  color: rgba(179, 177, 173, 0.45);
}

.atl-cal-weekday.last {
  border-right: none;
}

.atl-cal-grid {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-height: 0;
}

.atl-cal-week {
  flex: 1;
  display: flex;
  border-bottom: 1px solid var(--c-border);
  min-height: 0;
}

.atl-cal-week.last {
  border-bottom: none;
}

.atl-cal-cell {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
  padding: 5px 6px;
  border-right: 1px solid var(--c-border);
  overflow: hidden;
}

.atl-cal-cell.last {
  border-right: none;
}

.atl-cal-cell.weekend {
  background: rgba(15, 20, 25, 0.4);
}

.atl-cal-cell.today {
  background: rgba(255, 180, 84, 0.05);
}

.atl-cal-daynum-row {
  display: flex;
  justify-content: flex-end;
}

.atl-cal-daynum {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 20px;
  height: 20px;
  border-radius: var(--r-full);
  padding: 0 5px;
  font-size: 11px;
  font-weight: var(--fw-medium);
  font-family: var(--font-mono);
  color: var(--c-muted);
}

.atl-cal-daynum.out {
  color: rgba(179, 177, 173, 0.35);
}

.atl-cal-daynum.today {
  font-weight: var(--fw-bold);
  color: var(--c-primary-fg);
  background: var(--c-primary);
}

.atl-cal-chip {
  display: flex;
  align-items: center;
  gap: 5px;
  height: 19px;
  padding: 0 6px;
  border: none;
  border-radius: 3px;
  min-width: 0;
  cursor: pointer;
  text-align: left;
}

.atl-cal-chip.selected {
  box-shadow: inset 0 0 0 1px var(--c-primary);
}

.atl-cal-chipdot {
  flex: 0 0 auto;
  width: 5px;
  height: 5px;
  border-radius: var(--r-full);
}

.atl-cal-chiptitle {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 10.5px;
  color: var(--c-foreground);
}

.atl-cal-unsched {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
  padding: 8px 16px;
  border-top: 1px solid var(--c-border);
  background: var(--c-panel);
}

.atl-cal-unsched-label {
  font-size: 10px;
  font-weight: var(--fw-semibold);
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--c-muted);
}
</style>
