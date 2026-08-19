import type { PickerOption } from '@/components/tareas/TaskRowPicker.vue';
import { resolveColumnSwatchId } from '@/lib/columnColor';
import { swatchById } from '@/lib/swatches';
import { PRIORITY_COLOR, priorityLabel } from '@/lib/taskPriority';
import type { ColumnDto } from '@/stores/boards';

/**
 * Option lists for the inline task-row pickers (status / priority), built once
 * per distinct selection instead of once per row per render. A list row passes
 * its options down as a prop, so a fresh array on every render re-renders the
 * row and the three popovers it holds; these builders hand back the same array
 * for the same selection, which keeps the props stable.
 */

const PRIORITIES = ['urgent', 'high', 'medium', 'low'] as const;

function buildPriorityOptions(active: string): PickerOption[] {
  const options: PickerOption[] = PRIORITIES.map((priority) => ({
    value: priority,
    label: priorityLabel(priority),
    icon: 'flag',
    color: PRIORITY_COLOR[priority],
    active: priority === active,
  }));

  options.push({ value: '', label: 'Clear', icon: 'x', muted: true });
  return options;
}

const PRIORITY_OPTIONS_BY_ACTIVE = new Map<string, PickerOption[]>(
  ['', ...PRIORITIES].map((active) => [active, buildPriorityOptions(active)]),
);

const CLEARED_PRIORITY_OPTIONS = buildPriorityOptions('');

/** The priority picker's options, with the task's own priority marked active. */
export function priorityPickerOptions(priority: string | null | undefined): PickerOption[] {
  return PRIORITY_OPTIONS_BY_ACTIVE.get(priority ?? '') ?? CLEARED_PRIORITY_OPTIONS;
}

/**
 * Indexes a board's columns into one option list per possible active column, so
 * every row on the same column shares a single array. An id outside the board
 * (a stale column, a cross-board task) resolves to the all-inactive list.
 */
export function createStatusOptionsIndex(columns: ColumnDto[]): (activeColumnId: string) => PickerOption[] {
  const base: PickerOption[] = columns.map((column) => ({
    value: column.id,
    label: column.name,
    color: swatchById(resolveColumnSwatchId(column)).fg,
    active: false,
  }));

  const byActive = new Map<string, PickerOption[]>(
    columns.map((active) => [
      active.id,
      base.map((option) => ({ ...option, active: option.value === active.id })),
    ]),
  );

  return (activeColumnId) => byActive.get(activeColumnId) ?? base;
}
