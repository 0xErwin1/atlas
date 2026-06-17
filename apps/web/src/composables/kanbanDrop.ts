/**
 * Pure resolver for the SortableJS drop events emitted by vue-draggable-plus
 * (`@add` for a cross-column drop, `@update` for an in-column reorder). Both carry
 * the dragged DOM node as `item`; the task's `readable_id` is read from its
 * `data-readable-id` attribute (exposed by TaskCard), and the destination slot
 * from `newIndex`.
 *
 * vue-draggable-plus does NOT emit the `{ added, moved, removed }` `change` event
 * of the older `vuedraggable` library, so the drop must be read from these
 * SortableJS events instead.
 *
 * Returns the moving task's `readable_id` and the target index to feed into
 * `useKanbanMove.move`, or `null` when the event carries nothing actionable.
 */
export interface DropTarget {
  readableId: string;
  toIndex: number;
}

interface SortableDropEvent {
  item?: { dataset?: { readableId?: string } } | null;
  newIndex?: number | null;
}

export function resolveDropTarget(event: SortableDropEvent): DropTarget | null {
  const readableId = event.item?.dataset?.readableId;
  if (typeof readableId !== 'string' || readableId.length === 0) {
    return null;
  }

  const toIndex = typeof event.newIndex === 'number' ? event.newIndex : 0;
  return { readableId, toIndex };
}
