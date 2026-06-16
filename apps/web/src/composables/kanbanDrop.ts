/**
 * Pure resolver for the SortableJS `change` event emitted by vue-draggable-plus.
 *
 * The drag library fires `change` with exactly one of three shapes per drop:
 *   - `added`   — an item dropped INTO this column from another column.
 *   - `moved`   — an item reordered WITHIN this column.
 *   - `removed` — the source side of a cross-column move (handled by the
 *                 destination's `added`, so this side is a no-op).
 *
 * Returns the moving task's `readable_id` and the target index to feed into
 * `useKanbanMove.move`, or `null` when the event carries nothing actionable.
 */
export interface DropTarget {
  readableId: string;
  toIndex: number;
}

interface ChangePayload {
  element?: { readable_id?: unknown } | null;
  newIndex?: number;
}

interface SortableChangeEvent {
  added?: ChangePayload;
  moved?: ChangePayload;
  removed?: unknown;
}

function fromPayload(payload: ChangePayload | undefined): DropTarget | null {
  if (payload === undefined) {
    return null;
  }

  const readableId = payload.element?.readable_id;
  if (typeof readableId !== 'string' || readableId.length === 0) {
    return null;
  }

  const toIndex = typeof payload.newIndex === 'number' ? payload.newIndex : 0;
  return { readableId, toIndex };
}

export function resolveDropTarget(event: SortableChangeEvent): DropTarget | null {
  if (event.added !== undefined) {
    return fromPayload(event.added);
  }

  if (event.moved !== undefined) {
    return fromPayload(event.moved);
  }

  return null;
}
