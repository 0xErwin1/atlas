import { describe, expect, it } from 'vitest';
import { resolveDropTarget } from '@/composables/kanbanDrop';

describe('resolveDropTarget', () => {
  it('resolves an add event (cross-column drop) to the dropped item and its new index', () => {
    const event = {
      added: {
        element: { readable_id: 'ATL-42' },
        newIndex: 2,
      },
    };

    const target = resolveDropTarget(event);

    expect(target).toEqual({ readableId: 'ATL-42', toIndex: 2 });
  });

  it('resolves a moved event (within-column reorder) to the moved item and its new index', () => {
    const event = {
      moved: {
        element: { readable_id: 'ATL-7' },
        newIndex: 0,
      },
    };

    const target = resolveDropTarget(event);

    expect(target).toEqual({ readableId: 'ATL-7', toIndex: 0 });
  });

  it('ignores a removed event (the source side of a cross-column move)', () => {
    const event = {
      removed: {
        element: { readable_id: 'ATL-42' },
        oldIndex: 1,
      },
    };

    expect(resolveDropTarget(event)).toBeNull();
  });

  it('returns null for an event without a recognised change', () => {
    expect(resolveDropTarget({})).toBeNull();
  });

  it('returns null when the element lacks a readable_id', () => {
    expect(resolveDropTarget({ added: { element: {}, newIndex: 0 } })).toBeNull();
  });
});
