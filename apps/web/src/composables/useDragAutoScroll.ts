interface DragAutoScrollOptions {
  scroll: boolean;
  scrollSensitivity: number;
  scrollSpeed: number;
  bubbleScroll: boolean;
  forceAutoScrollFallback: boolean;
}

export interface DragAutoScrollPointer {
  clientX: number;
  clientY: number;
}

interface DragAutoScrollBehaviorOptions {
  edgeThreshold: number;
  scrollStep: number;
}

export interface DragAutoScrollMoveEvent {
  to: HTMLElement;
}

const dragAutoScrollBehaviorDefaults: DragAutoScrollBehaviorOptions = {
  edgeThreshold: 60,
  scrollStep: 14,
};

export const dragAutoScrollOptions = {
  scroll: true,
  scrollSensitivity: dragAutoScrollBehaviorDefaults.edgeThreshold,
  scrollSpeed: dragAutoScrollBehaviorDefaults.scrollStep,
  bubbleScroll: true,
  forceAutoScrollFallback: true,
} satisfies DragAutoScrollOptions;

export function applyDragAutoScroll(
  element: HTMLElement,
  pointer: DragAutoScrollPointer,
  options: DragAutoScrollBehaviorOptions = dragAutoScrollBehaviorDefaults,
): boolean {
  const rect = element.getBoundingClientRect();
  let topDelta = 0;
  let leftDelta = 0;

  if (pointer.clientY - rect.top <= options.edgeThreshold) {
    topDelta = -options.scrollStep;
  } else if (rect.bottom - pointer.clientY <= options.edgeThreshold) {
    topDelta = options.scrollStep;
  }

  if (pointer.clientX - rect.left <= options.edgeThreshold) {
    leftDelta = -options.scrollStep;
  } else if (rect.right - pointer.clientX <= options.edgeThreshold) {
    leftDelta = options.scrollStep;
  }

  if (topDelta === 0 && leftDelta === 0) {
    return false;
  }

  const beforeTop = element.scrollTop;
  const beforeLeft = element.scrollLeft;
  element.scrollTop += topDelta;
  element.scrollLeft += leftDelta;

  return element.scrollTop !== beforeTop || element.scrollLeft !== beforeLeft;
}

export function handleDragAutoScrollMove(
  event: DragAutoScrollMoveEvent,
  originalEvent: Event,
  explicitContainer?: HTMLElement | null,
): boolean {
  const pointer = pointerFromEvent(originalEvent);
  if (pointer === null) return false;

  const container = explicitContainer ?? findScrollableAncestor(event.to);
  if (container === null) return false;

  return applyDragAutoScroll(container, pointer);
}

function pointerFromEvent(event: Event): DragAutoScrollPointer | null {
  const candidate = event as Partial<DragAutoScrollPointer>;
  if (typeof candidate.clientX === 'number' && typeof candidate.clientY === 'number') {
    return { clientX: candidate.clientX, clientY: candidate.clientY };
  }

  const touchEvent = event as Partial<Pick<TouchEvent, 'touches' | 'changedTouches'>>;
  const touch = touchEvent.touches?.[0] ?? touchEvent.changedTouches?.[0];
  if (touch !== undefined) {
    return { clientX: touch.clientX, clientY: touch.clientY };
  }

  return null;
}

function findScrollableAncestor(start: HTMLElement | null): HTMLElement | null {
  for (let element = start; element !== null; element = element.parentElement) {
    if (element.scrollHeight > element.clientHeight || element.scrollWidth > element.clientWidth) {
      return element;
    }
  }
  return null;
}
