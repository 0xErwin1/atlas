import { describe, expect, it } from 'vitest';
import { resolveDropTarget } from '@/composables/kanbanDrop';

describe('resolveDropTarget', () => {
  it('resolves a SortableJS drop event to the dragged item and its new index', () => {
    const event = {
      item: { dataset: { readableId: 'ATL-42' } },
      newIndex: 2,
    };

    const target = resolveDropTarget(event);

    expect(target).toEqual({ readableId: 'ATL-42', toIndex: 2 });
  });

  it('defaults the index to 0 when newIndex is missing', () => {
    const event = {
      item: { dataset: { readableId: 'ATL-7' } },
    };

    const target = resolveDropTarget(event);

    expect(target).toEqual({ readableId: 'ATL-7', toIndex: 0 });
  });

  it('returns null when the dragged item has no readable id', () => {
    expect(resolveDropTarget({ item: { dataset: {} }, newIndex: 0 })).toBeNull();
  });

  it('returns null for an event without an item', () => {
    expect(resolveDropTarget({})).toBeNull();
  });
});
