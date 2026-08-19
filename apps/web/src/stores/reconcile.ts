/**
 * Identity-preserving reconciliation for store state.
 *
 * Every revalidation hands a store freshly parsed JSON, so replacing state
 * wholesale gives every object a new identity even when nothing changed. Vue
 * then re-patches every consumer, and the kanban's list-identity memoization
 * (see `boards.ts`) loses its cache. These helpers keep the object that is
 * already in the store whenever the incoming value is deeply equal to it, so a
 * no-op publish is a no-op for the DOM as well.
 */

/** Structural equality over JSON-shaped data (DTOs). Non-plain values compare by identity. */
export function deepEqual(left: unknown, right: unknown): boolean {
  if (Object.is(left, right)) return true;
  if (typeof left !== 'object' || typeof right !== 'object' || left === null || right === null) return false;

  if (Array.isArray(left) || Array.isArray(right)) {
    if (!Array.isArray(left) || !Array.isArray(right) || left.length !== right.length) return false;
    return left.every((item, index) => deepEqual(item, right[index]));
  }

  const leftRecord = left as Record<string, unknown>;
  const rightRecord = right as Record<string, unknown>;
  const leftKeys = Object.keys(leftRecord);
  if (leftKeys.length !== Object.keys(rightRecord).length) return false;

  return leftKeys.every(
    (key) => Object.hasOwn(rightRecord, key) && deepEqual(leftRecord[key], rightRecord[key]),
  );
}

/** Returns `current` when `next` is deeply equal to it, so the reference survives. */
export function reconcileValue<T>(current: T, next: T): T {
  return deepEqual(current, next) ? current : next;
}

/**
 * Reconciles a list against its incoming replacement.
 *
 * Returns `current` untouched when the two are deeply equal. Otherwise returns a
 * new array that reuses every element the store already holds unchanged; `keyOf`
 * lets an element be recognized after a reorder or an insertion, falling back to
 * positional matching when it is absent.
 */
export function reconcileList<T>(current: T[], next: T[], keyOf?: (item: T) => string): T[] {
  const byKey = keyOf === undefined ? null : new Map(current.map((item) => [keyOf(item), item]));

  let changed = current.length !== next.length;

  const result = next.map((item, index) => {
    const candidate = (keyOf === undefined ? undefined : byKey?.get(keyOf(item))) ?? current[index];
    if (candidate !== undefined && deepEqual(candidate, item)) {
      if (candidate !== current[index]) changed = true;
      return candidate;
    }

    changed = true;
    return item;
  });

  return changed ? result : current;
}

/**
 * Reconciles a map of lists (tasks per column). Returns `current` when every
 * bucket and the key set are unchanged, so a background board refresh does not
 * hand the kanban a new Map for identical data.
 */
export function reconcileListMap<T>(
  current: Map<string, T[]>,
  next: Map<string, T[]>,
  keyOf?: (item: T) => string,
): Map<string, T[]> {
  let changed = current.size !== next.size;
  const result = new Map<string, T[]>();

  for (const [key, list] of next) {
    const existing = current.get(key);
    const reconciled = reconcileList(existing ?? [], list, keyOf);
    if (reconciled !== existing) changed = true;
    result.set(key, reconciled);
  }

  return changed ? result : current;
}
