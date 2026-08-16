import type { TaskSummaryDto } from '@/stores/boards';

/**
 * Column sorting for the flat task list.
 *
 * Kept out of the view so the ordering rules can be read and tested on their
 * own: a task list is scanned, and a sort that quietly disagrees with what the
 * column shows is worse than no sort at all.
 */

export type TaskSortKey =
  | 'name'
  | 'board'
  | 'id'
  | 'assignee'
  | 'priority'
  | 'status'
  | 'estimate'
  | 'updated';

export type SortDirection = 'asc' | 'desc';

/** Priority ordered by urgency, so a numeric comparison means what it reads like. */
const PRIORITY_RANK: Record<string, number> = {
  low: 1,
  medium: 2,
  high: 3,
  urgent: 4,
};

/**
 * The direction a column sorts on its first click.
 *
 * Text reads best from A; a priority, an estimate or a timestamp is asked about
 * from the top — nobody clicks "Updated" to see the stalest task first.
 */
export function defaultDirectionFor(key: TaskSortKey): SortDirection {
  return key === 'priority' || key === 'estimate' || key === 'updated' ? 'desc' : 'asc';
}

/**
 * Splits a readable id into its prefix and number so `ATL-9` sorts before
 * `ATL-10`. A plain string comparison would put 10 first, which reads as a bug
 * to anyone scanning the column.
 */
function readableIdParts(readableId: string): [string, number] {
  const match = /^(.*?)(\d+)$/.exec(readableId);
  if (match === null) return [readableId.toLowerCase(), 0];
  return [(match[1] ?? '').toLowerCase(), Number(match[2])];
}

function assigneeName(task: TaskSummaryDto): string | null {
  const actor = task.assignees?.[0];
  if (actor === undefined) return null;
  return (actor.display_name ?? '').toLowerCase();
}

function estimateOf(task: TaskSummaryDto): number | null {
  return typeof task.estimate === 'number' ? task.estimate : null;
}

function updatedAt(task: TaskSummaryDto): number | null {
  const parsed = Date.parse(task.updated_at ?? '');
  return Number.isNaN(parsed) ? null : parsed;
}

function priorityRank(task: TaskSummaryDto): number | null {
  const priority = (task.priority ?? '').toLowerCase();
  return PRIORITY_RANK[priority] ?? null;
}

function compareText(a: string, b: string): number {
  return a.localeCompare(b, undefined, { numeric: true, sensitivity: 'base' });
}

/**
 * Compares two tasks on `key`, or returns null when neither carries the value.
 *
 * A task with no assignee, estimate, priority or timestamp has nothing to place
 * in that order, so it is reported as absent and the caller pushes it to the
 * end — in both directions. Reversing "no priority" into first place would make
 * the descending view start with the tasks the sort is least about.
 */
function compareOn(a: TaskSummaryDto, b: TaskSummaryDto, key: TaskSortKey): number | 'both-absent' {
  switch (key) {
    case 'name':
      return compareText(a.title, b.title);
    case 'board':
      return compareText(a.board_name ?? '', b.board_name ?? '');
    case 'status':
      return compareText(a.column_name ?? '', b.column_name ?? '');
    case 'id': {
      const [prefixA, numberA] = readableIdParts(a.readable_id);
      const [prefixB, numberB] = readableIdParts(b.readable_id);
      return prefixA === prefixB ? numberA - numberB : compareText(prefixA, prefixB);
    }
    case 'assignee':
      return compareOptional(assigneeName(a), assigneeName(b), compareText);
    case 'priority':
      return compareOptional(priorityRank(a), priorityRank(b), (x, y) => x - y);
    case 'estimate':
      return compareOptional(estimateOf(a), estimateOf(b), (x, y) => x - y);
    case 'updated':
      return compareOptional(updatedAt(a), updatedAt(b), (x, y) => x - y);
  }
}

/** Absent values report themselves so the caller can keep them last. */
function compareOptional<T>(
  a: T | null,
  b: T | null,
  compare: (a: T, b: T) => number,
): number | 'both-absent' {
  if (a === null && b === null) return 'both-absent';
  if (a === null) return 1;
  if (b === null) return -1;
  return compare(a, b);
}

/**
 * Returns a copy of `tasks` ordered by `key`.
 *
 * `key = null` returns the incoming order untouched, which is how a column
 * un-sorts. Ties keep the incoming order (the sort is stable), so a second
 * column never silently reshuffles rows that compare equal.
 */
export function sortTasks(
  tasks: TaskSummaryDto[],
  key: TaskSortKey | null,
  direction: SortDirection,
): TaskSummaryDto[] {
  if (key === null) return [...tasks];

  const factor = direction === 'asc' ? 1 : -1;
  const absent = new Set<TaskSummaryDto>();
  const placed: TaskSummaryDto[] = [];

  for (const task of tasks) {
    if (compareOn(task, task, key) === 'both-absent') absent.add(task);
    else placed.push(task);
  }

  placed.sort((a, b) => {
    const result = compareOn(a, b, key);
    return result === 'both-absent' ? 0 : result * factor;
  });

  return [...placed, ...absent];
}
