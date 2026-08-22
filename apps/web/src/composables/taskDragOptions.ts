import { dragAutoScrollOptions } from '@/composables/useDragAutoScroll';

interface DragTouchOptions {
  delay: number;
  delayOnTouchOnly: boolean;
  touchStartThreshold: number;
}

/**
 * SortableJS options that keep a finger scroll from becoming a drag.
 *
 * Touch events never go through native HTML5 drag-and-drop, so SortableJS would
 * otherwise claim the gesture on `touchstart` and every swipe over a task
 * would move it. With these options a touch drag starts only after the finger
 * holds still for `delay` ms; moving past `touchStartThreshold` px before that
 * cancels the pending drag and leaves the scroll to the browser. Mouse and pen
 * drags are unaffected (`delayOnTouchOnly`).
 */
export const dragTouchOptions = {
  delay: 150,
  delayOnTouchOnly: true,
  touchStartThreshold: 5,
} satisfies DragTouchOptions;

/**
 * The SortableJS options every task drag surface (kanban column, task list)
 * binds: edge auto-scroll plus the touch hold above.
 */
export const taskDragOptions = {
  ...dragAutoScrollOptions,
  ...dragTouchOptions,
};
