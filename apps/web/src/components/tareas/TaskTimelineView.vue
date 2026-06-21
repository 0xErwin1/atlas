<script setup lang="ts">
/**
 * Task Timeline (Gantt) view: a day axis around today with one bar per task,
 * mirroring the hi-fi Timeline design (taskboards.jsx).
 *
 * Data reality: the model has a due date but NO start date. So a bar's END is
 * the task's real due date, while its start is a fixed presentational lead
 * (PRESENTATIONAL_LEAD_DAYS) before the due — it is NOT real scheduling data.
 * Tasks with no due date cannot be placed and are omitted (their count is shown).
 * Status color is the user's registry choice for the task's column value.
 */
import { computed } from 'vue';
import Avatar from '@/components/ui/Avatar.vue';
import { type Swatch, swatchById } from '@/lib/swatches';
import type { TaskSummaryDto } from '@/stores/boards';
import { useBoardsStore } from '@/stores/boards';
import { useLabelColorsStore } from '@/stores/labelColors';

defineProps<{ ws: string; selectedReadableId: string | null }>();
const emit = defineEmits<{ select: [readableId: string]; open: [readableId: string] }>();

const boards = useBoardsStore();
const labelColors = useLabelColorsStore();

const WEEKDAYS = ['M', 'T', 'W', 'T', 'F', 'S', 'S'];
const NAME_W = 300;
const WINDOW_LEAD_DAYS = 7;
const WINDOW_DAYS = 21;
// No start_date exists in the data model, so a bar spans this many days before
// its real due date purely so it reads as a bar instead of a point.
const PRESENTATIONAL_LEAD_DAYS = 2;
const MS_PER_DAY = 86_400_000;

const today = new Date();
const todayMidnight = new Date(today.getFullYear(), today.getMonth(), today.getDate());
const axisStart = new Date(todayMidnight.getTime() - WINDOW_LEAD_DAYS * MS_PER_DAY);

interface AxisDay {
  index: number;
  day: number;
  letter: string;
  isToday: boolean;
}

const axisDays = computed<AxisDay[]>(() =>
  Array.from({ length: WINDOW_DAYS }, (_, i) => {
    const date = new Date(axisStart.getTime() + i * MS_PER_DAY);
    return {
      index: i,
      day: date.getDate(),
      letter: WEEKDAYS[(date.getDay() + 6) % 7] ?? '',
      isToday: date.getTime() === todayMidnight.getTime(),
    };
  }),
);

const todayPct = computed(() => ((WINDOW_LEAD_DAYS + 0.5) / WINDOW_DAYS) * 100);

const allTasks = computed<TaskSummaryDto[]>(() =>
  boards.columns.flatMap((c) => boards.filteredTasksByColumn(c.id)),
);

function dueIndex(readableId: string): number | null {
  const raw = boards.taskDetail(readableId)?.due_date;
  if (raw === null || raw === undefined) return null;

  const due = new Date(raw);
  if (Number.isNaN(due.getTime())) return null;

  const dueMidnight = new Date(due.getFullYear(), due.getMonth(), due.getDate());
  return Math.round((dueMidnight.getTime() - axisStart.getTime()) / MS_PER_DAY);
}

function statusSwatch(task: TaskSummaryDto): Swatch {
  const column = boards.columns.find((c) => c.id === task.column_id);
  return swatchById(labelColors.colorFor(`status:${column?.name ?? ''}`));
}

function firstAssignee(task: TaskSummaryDto): { name: string; agent: boolean } | null {
  const actor = task.assignees?.[0];
  if (actor === undefined) return null;
  const agent = actor.type === 'api_key';
  return { name: actor.display_name ?? (agent ? 'Agent' : 'User'), agent };
}

interface Bar {
  task: TaskSummaryDto;
  swatch: Swatch;
  leftPct: number;
  widthPct: number;
  estimate: string;
}

const bars = computed<Bar[]>(() => {
  const placed: { idx: number; bar: Bar }[] = [];

  for (const task of allTasks.value) {
    const idx = dueIndex(task.readable_id);
    if (idx === null) continue;

    const start = idx - PRESENTATIONAL_LEAD_DAYS;
    if (idx < 0 || start > WINDOW_DAYS - 1) continue;

    const clampedStart = Math.max(start, 0);
    const clampedEnd = Math.min(idx, WINDOW_DAYS - 1);

    placed.push({
      idx,
      bar: {
        task,
        swatch: statusSwatch(task),
        leftPct: (clampedStart / WINDOW_DAYS) * 100,
        widthPct: ((clampedEnd - clampedStart + 1) / WINDOW_DAYS) * 100,
        estimate:
          task.estimate !== null && task.estimate !== undefined ? `${task.estimate} pts` : task.readable_id,
      },
    });
  }

  return placed.sort((a, b) => a.idx - b.idx).map((p) => p.bar);
});

const offWindowCount = computed(() => {
  const total = allTasks.value.length;
  const withDue = allTasks.value.filter((t) => dueIndex(t.readable_id) !== null).length;
  return { undated: total - withDue, total };
});
</script>

<template>
  <div class="atl-tm">
    <div class="atl-tm-inner" :style="{ minWidth: `${NAME_W + WINDOW_DAYS * 52}px` }">
      <div class="atl-tm-axis">
        <div class="atl-tm-axis-name" :style="{ width: `${NAME_W}px`, flex: `0 0 ${NAME_W}px` }">Task</div>
        <div class="flex" style="flex: 1;">
          <div
            v-for="d in axisDays"
            :key="d.index"
            class="atl-tm-axis-day"
            :class="{ today: d.isToday }"
          >
            <span class="atl-tm-axis-letter">{{ d.letter }}</span>
            <span class="atl-tm-axis-num" :class="{ today: d.isToday }">{{ d.day }}</span>
          </div>
        </div>
      </div>

      <div class="atl-tm-rows">
        <div class="atl-tm-today" :style="{ left: `calc(${NAME_W}px + (100% - ${NAME_W}px) * ${todayPct / 100})` }" />

        <div v-for="bar in bars" :key="bar.task.id" class="atl-tm-row">
          <div class="atl-tm-name" :style="{ width: `${NAME_W}px`, flex: `0 0 ${NAME_W}px` }">
            <span class="atl-tm-dot" :style="{ background: bar.swatch.fg }" />
            <span class="atl-tm-title">{{ bar.task.title }}</span>
            <span class="atl-tm-id">{{ bar.task.readable_id }}</span>
          </div>
          <div class="atl-tm-track">
            <button
              type="button"
              class="atl-tm-bar"
              :class="{ selected: bar.task.readable_id === selectedReadableId }"
              :style="{
                left: `${bar.leftPct}%`,
                width: `${bar.widthPct}%`,
                background: bar.swatch.bg,
                borderColor: bar.swatch.fg,
              }"
              :title="bar.task.title"
              @click="emit('select', bar.task.readable_id)"
            >
              <span class="atl-tm-est" :style="{ color: bar.swatch.fg }">{{ bar.estimate }}</span>
              <Avatar
                v-if="firstAssignee(bar.task)"
                :name="firstAssignee(bar.task)!.name"
                :agent="firstAssignee(bar.task)!.agent"
                :size="16"
              />
            </button>
          </div>
        </div>

        <div v-if="bars.length === 0" class="atl-tm-empty">
          <span v-if="boards.detailsLoading">Loading due dates…</span>
          <span v-else>No tasks with a due date in this range.</span>
        </div>
      </div>

      <div v-if="offWindowCount.undated > 0" class="atl-tm-note">
        {{ offWindowCount.undated }} task{{ offWindowCount.undated === 1 ? '' : 's' }} with no due date are not shown.
      </div>
    </div>
  </div>
</template>

<style scoped>
.atl-tm {
  flex: 1 1 0;
  min-width: 0;
  min-height: 0;
  overflow: auto;
  background: var(--c-background);
}

.atl-tm-axis {
  display: flex;
  height: 38px;
  position: sticky;
  top: 0;
  z-index: 2;
  background: var(--c-panel);
  border-bottom: 1px solid var(--c-border);
}

.atl-tm-axis-name {
  display: flex;
  align-items: center;
  padding: 0 14px;
  font-size: 11px;
  font-weight: var(--fw-semibold);
  letter-spacing: 0.04em;
  text-transform: uppercase;
  color: var(--c-muted);
  border-right: 1px solid var(--c-border);
}

.atl-tm-axis-day {
  flex: 1;
  min-width: 40px;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  border-right: 1px solid var(--c-border);
}

.atl-tm-axis-day.today {
  background: rgba(255, 180, 84, 0.06);
}

.atl-tm-axis-letter {
  font-size: 9px;
  color: rgba(179, 177, 173, 0.5);
  text-transform: uppercase;
}

.atl-tm-axis-num {
  font-family: var(--font-mono);
  font-size: 11px;
  font-weight: var(--fw-medium);
  color: var(--c-muted);
}

.atl-tm-axis-num.today {
  font-weight: var(--fw-bold);
  color: var(--c-primary);
}

.atl-tm-rows {
  position: relative;
}

.atl-tm-today {
  position: absolute;
  top: 0;
  bottom: 0;
  width: 2px;
  background: rgba(255, 180, 84, 0.55);
  z-index: 3;
  pointer-events: none;
}

.atl-tm-row {
  display: flex;
  height: 44px;
  border-bottom: 1px solid var(--c-border);
}

.atl-tm-name {
  display: flex;
  align-items: center;
  gap: 9px;
  padding: 0 14px;
  border-right: 1px solid var(--c-border);
  min-width: 0;
}

.atl-tm-dot {
  flex: 0 0 auto;
  width: 7px;
  height: 7px;
  border-radius: var(--r-full);
}

.atl-tm-title {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12.5px;
  color: var(--c-foreground);
}

.atl-tm-id {
  font-family: var(--font-mono);
  font-size: 10.5px;
  color: var(--c-muted);
}

.atl-tm-track {
  flex: 1;
  position: relative;
}

.atl-tm-bar {
  position: absolute;
  top: 50%;
  transform: translateY(-50%);
  height: 24px;
  border-radius: 4px;
  border: 1px solid;
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 0 7px;
  overflow: hidden;
  cursor: pointer;
}

.atl-tm-bar.selected {
  box-shadow: 0 0 0 1px var(--c-primary);
}

.atl-tm-est {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 10.5px;
  font-weight: var(--fw-semibold);
}

.atl-tm-empty {
  padding: 24px 14px;
  font-size: var(--fs-sm);
  color: var(--c-muted);
}

.atl-tm-note {
  padding: 8px 14px;
  font-size: var(--fs-xs);
  color: var(--c-muted);
}
</style>
