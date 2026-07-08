import { describe, expect, it } from 'vitest';
import {
  applyDragAutoScroll,
  type DragAutoScrollPointer,
  dragAutoScrollOptions,
} from '@/composables/useDragAutoScroll';

describe('dragAutoScrollOptions', () => {
  it('configures SortableJS to scroll parent containers while dragging near edges', () => {
    expect(dragAutoScrollOptions).toEqual({
      scroll: true,
      scrollSensitivity: 60,
      scrollSpeed: 14,
      bubbleScroll: true,
      forceAutoScrollFallback: true,
    });
  });
});

describe('applyDragAutoScroll', () => {
  function scrollBox(pointer: DragAutoScrollPointer = { clientX: 100, clientY: 100 }): HTMLElement {
    const element = document.createElement('div');
    Object.defineProperties(element, {
      clientHeight: { configurable: true, value: 200 },
      clientWidth: { configurable: true, value: 200 },
      scrollHeight: { configurable: true, value: 400 },
      scrollWidth: { configurable: true, value: 400 },
    });
    element.scrollTop = 100;
    element.scrollLeft = 100;
    element.getBoundingClientRect = () => ({
      x: 0,
      y: 0,
      top: 0,
      left: 0,
      right: 200,
      bottom: 200,
      width: 200,
      height: 200,
      toJSON: () => ({}),
    });
    applyDragAutoScroll(element, pointer);
    return element;
  }

  it('scrolls down when the pointer is near the bottom edge', () => {
    const element = scrollBox({ clientX: 100, clientY: 196 });

    expect(element.scrollTop).toBe(114);
    expect(element.scrollLeft).toBe(100);
  });

  it('scrolls up when the pointer is near the top edge', () => {
    const element = scrollBox({ clientX: 100, clientY: 4 });

    expect(element.scrollTop).toBe(86);
  });

  it('scrolls horizontally near left and right edges', () => {
    const right = scrollBox({ clientX: 196, clientY: 100 });
    expect(right.scrollLeft).toBe(114);

    const left = scrollBox({ clientX: 4, clientY: 100 });
    expect(left.scrollLeft).toBe(86);
  });

  it('does not scroll when the pointer stays in the safe center zone', () => {
    const element = scrollBox({ clientX: 100, clientY: 100 });

    expect(element.scrollTop).toBe(100);
    expect(element.scrollLeft).toBe(100);
  });
});
