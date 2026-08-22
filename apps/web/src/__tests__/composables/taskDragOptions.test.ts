import { describe, expect, it } from 'vitest';
import { dragTouchOptions, taskDragOptions } from '@/composables/taskDragOptions';
import { dragAutoScrollOptions } from '@/composables/useDragAutoScroll';

describe('taskDragOptions', () => {
  it('combines edge auto-scroll with the touch hold for every task drag surface', () => {
    expect(taskDragOptions).toEqual({ ...dragAutoScrollOptions, ...dragTouchOptions });
  });
});

describe('dragTouchOptions', () => {
  it('requires a touch hold before a drag starts so a finger scroll is never a drag', () => {
    expect(dragTouchOptions).toEqual({
      delay: 150,
      delayOnTouchOnly: true,
      touchStartThreshold: 5,
    });
  });
});
