<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import Icon from '@/components/ui/Icon.vue';
import Popover from '@/components/ui/Popover.vue';
import { formatDate } from '@/lib/format';

/**
 * Optional calendar date picker built on the shared `Popover` primitive, so it
 * inherits click-outside dismissal, Escape handling and the app surface styling
 * instead of the unstyled, non-dismissible native `<input type="date">` popover.
 *
 * The value is a `YYYY-MM-DD` string, with the empty string meaning "no date"
 * (e.g. an API key that never expires). Keeping the empty-string / date-string
 * shape lets callers preserve the existing `value === '' ? null : …` contract
 * unchanged. The picker never emits a partial or invalid date.
 */

const props = withDefaults(
  defineProps<{
    /** Label shown on the trigger when no date is selected. */
    placeholder?: string;
    /** Label of the in-panel button that resets the value to unset. */
    clearLabel?: string;
    disabled?: boolean;
  }>(),
  {
    placeholder: 'No date',
    clearLabel: 'Clear',
    disabled: false,
  },
);

const model = defineModel<string>({ default: '' });

const WEEKDAYS = [
  { short: 'S', long: 'Sunday' },
  { short: 'M', long: 'Monday' },
  { short: 'T', long: 'Tuesday' },
  { short: 'W', long: 'Wednesday' },
  { short: 'T', long: 'Thursday' },
  { short: 'F', long: 'Friday' },
  { short: 'S', long: 'Saturday' },
];

const MONTHS = [
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

function pad(n: number): string {
  return n < 10 ? `0${n}` : `${n}`;
}

/** Builds a `YYYY-MM-DD` string from a 0-based month. */
function toDateString(year: number, month: number, day: number): string {
  return `${year}-${pad(month + 1)}-${pad(day)}`;
}

const selected = computed(() => {
  if (model.value === '') return null;

  const [year, month, day] = model.value.split('-').map(Number);
  if (!year || !month || !day) return null;

  return { year, month: month - 1, day };
});

const hasValue = computed(() => model.value !== '');

// Local noon keeps the label on the correct calendar day in every timezone; a
// bare `YYYY-MM-DD` parses as UTC midnight and can render as the previous day.
const displayLabel = computed(() =>
  model.value === '' ? props.placeholder : formatDate(`${model.value}T12:00:00`),
);

const today = new Date();

const viewYear = ref(selected.value?.year ?? today.getFullYear());
const viewMonth = ref(selected.value?.month ?? today.getMonth());

watch(selected, (value) => {
  if (value !== null) {
    viewYear.value = value.year;
    viewMonth.value = value.month;
  }
});

const monthLabel = computed(() => `${MONTHS[viewMonth.value]} ${viewYear.value}`);

const cells = computed<(number | null)[]>(() => {
  const firstWeekday = new Date(viewYear.value, viewMonth.value, 1).getDay();
  const daysInMonth = new Date(viewYear.value, viewMonth.value + 1, 0).getDate();

  const grid: (number | null)[] = [];

  for (let i = 0; i < firstWeekday; i += 1) grid.push(null);
  for (let day = 1; day <= daysInMonth; day += 1) grid.push(day);

  return grid;
});

function prevMonth(): void {
  if (viewMonth.value === 0) {
    viewMonth.value = 11;
    viewYear.value -= 1;
    return;
  }
  viewMonth.value -= 1;
}

function nextMonth(): void {
  if (viewMonth.value === 11) {
    viewMonth.value = 0;
    viewYear.value += 1;
    return;
  }
  viewMonth.value += 1;
}

function isSelected(day: number): boolean {
  const value = selected.value;
  return (
    value !== null && value.year === viewYear.value && value.month === viewMonth.value && value.day === day
  );
}

function isToday(day: number): boolean {
  return (
    today.getFullYear() === viewYear.value && today.getMonth() === viewMonth.value && today.getDate() === day
  );
}

function dayLabel(day: number): string {
  return formatDate(`${toDateString(viewYear.value, viewMonth.value, day)}T12:00:00`);
}

function pick(day: number, close: () => void): void {
  model.value = toDateString(viewYear.value, viewMonth.value, day);
  close();
}

function clear(close: () => void): void {
  model.value = '';
  close();
}
</script>

<template>
  <Popover placement="bottom-start" width="248px" block teleport>
    <template #trigger="{ open, toggle }">
      <button
        type="button"
        class="atl-dp-trigger"
        :class="{ 'atl-dp-placeholder': !hasValue }"
        :disabled="disabled"
        aria-haspopup="dialog"
        :aria-expanded="open"
        @click="toggle"
      >
        <Icon name="calendar" :size="14" class="atl-dp-lead" />
        <span class="atl-dp-label">{{ displayLabel }}</span>
        <Icon
          name="chevron-down"
          :size="12"
          class="atl-dp-chevron"
          :style="{ transform: open ? 'rotate(180deg)' : 'none' }"
        />
      </button>
    </template>

    <template #default="{ close }">
      <div class="atl-dp-panel" role="dialog" aria-label="Choose a date">
        <div class="atl-dp-header">
          <button
            type="button"
            class="atl-dp-nav"
            aria-label="Previous month"
            @click="prevMonth"
          >
            <Icon name="chevron-left" :size="16" />
          </button>
          <span class="atl-dp-month" aria-live="polite">{{ monthLabel }}</span>
          <button
            type="button"
            class="atl-dp-nav"
            aria-label="Next month"
            @click="nextMonth"
          >
            <Icon name="chevron-right" :size="16" />
          </button>
        </div>

        <div class="atl-dp-weekdays" aria-hidden="true">
          <span v-for="(weekday, i) in WEEKDAYS" :key="i" :title="weekday.long">
            {{ weekday.short }}
          </span>
        </div>

        <div class="atl-dp-grid" role="grid">
          <template v-for="(day, i) in cells" :key="i">
            <span v-if="day === null" class="atl-dp-empty" />
            <button
              v-else
              type="button"
              class="atl-dp-day"
              :class="{
                'atl-dp-day--selected': isSelected(day),
                'atl-dp-day--today': isToday(day),
              }"
              role="gridcell"
              :aria-label="dayLabel(day)"
              :aria-pressed="isSelected(day)"
              @click="pick(day, close)"
            >
              {{ day }}
            </button>
          </template>
        </div>

        <button type="button" class="atl-dp-clear" @click="clear(close)">
          <Icon name="x" :size="13" />
          {{ clearLabel }}
        </button>
      </div>
    </template>
  </Popover>
</template>

<style scoped>
.atl-dp-trigger {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  height: var(--h-input);
  padding: 0 10px;
  background-color: var(--c-input);
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  color: var(--c-foreground);
  font-size: var(--fs-base);
  cursor: pointer;
  text-align: left;
}

.atl-dp-trigger:disabled {
  opacity: 0.55;
  cursor: not-allowed;
}

.atl-dp-placeholder .atl-dp-label {
  color: var(--c-muted);
}

.atl-dp-lead {
  color: var(--c-muted);
  flex: 0 0 auto;
}

.atl-dp-label {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.atl-dp-chevron {
  color: var(--c-muted);
  flex: 0 0 auto;
  transition: transform 0.1s;
}

.atl-dp-panel {
  padding: 8px;
}

.atl-dp-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 6px;
}

.atl-dp-month {
  font-size: var(--fs-sm);
  font-weight: var(--fw-semibold);
  color: var(--c-foreground);
}

.atl-dp-nav {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border: none;
  border-radius: var(--r-sm);
  background: transparent;
  color: var(--c-muted);
  cursor: pointer;
}

.atl-dp-nav:hover {
  background: var(--c-selection);
  color: var(--c-foreground);
}

.atl-dp-weekdays,
.atl-dp-grid {
  display: grid;
  grid-template-columns: repeat(7, 1fr);
}

.atl-dp-weekdays {
  margin-bottom: 2px;
}

.atl-dp-weekdays span {
  display: flex;
  align-items: center;
  justify-content: center;
  height: 24px;
  font-size: 10px;
  font-weight: var(--fw-semibold);
  color: var(--c-muted);
}

.atl-dp-empty {
  height: 30px;
}

.atl-dp-day {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  height: 30px;
  border: none;
  border-radius: var(--r-sm);
  background: transparent;
  color: var(--c-foreground);
  font-size: var(--fs-sm);
  cursor: pointer;
}

.atl-dp-day:hover {
  background: var(--c-selection);
}

.atl-dp-day--today {
  font-weight: var(--fw-semibold);
  color: var(--c-primary);
}

.atl-dp-day--selected,
.atl-dp-day--selected:hover {
  background: var(--c-primary);
  color: var(--c-primary-fg);
}

.atl-dp-clear {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  width: 100%;
  height: 28px;
  margin-top: 6px;
  border: none;
  border-top: 1px solid var(--c-border);
  border-radius: 0;
  padding-top: 8px;
  background: transparent;
  color: var(--c-muted);
  font-size: var(--fs-sm);
  cursor: pointer;
}

.atl-dp-clear:hover {
  color: var(--c-foreground);
}
</style>
