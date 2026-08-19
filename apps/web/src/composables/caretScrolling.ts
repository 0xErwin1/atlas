interface VerticalSpan {
  top: number;
  bottom: number;
}

/**
 * How far the scroll container must move for the caret to clear the margin at
 * the top and bottom of its visible box. Zero while the caret is already inside
 * it, so a keystroke in the middle of the page writes no scroll position.
 */
export function caretScrollDelta(caret: VerticalSpan, box: VerticalSpan, margin: number): number {
  if (caret.bottom > box.bottom - margin) return caret.bottom - (box.bottom - margin);
  if (caret.top < box.top + margin) return caret.top - (box.top + margin);
  return 0;
}

function findScrollableAncestor(el: HTMLElement): HTMLElement | null {
  let current = el.parentElement;

  while (current !== null) {
    const overflowY = getComputedStyle(current).overflowY;
    if ((overflowY === 'auto' || overflowY === 'scroll') && current.scrollHeight > current.clientHeight) {
      return current;
    }
    current = current.parentElement;
  }

  return null;
}

/**
 * Resolves the closest ancestor that actually scrolls vertically, remembering
 * the answer. The walk costs one `getComputedStyle` per level, and it runs on
 * every keystroke, so it is repeated only when the element is re-parented or
 * when no ancestor scrolled yet — a container that has not overflowed can still
 * become the scroller once the document grows.
 */
export function createScrollableAncestorResolver(): (el: HTMLElement) => HTMLElement | null {
  let cachedParent: HTMLElement | null = null;
  let cached: HTMLElement | null = null;

  return (el) => {
    const parent = el.parentElement;
    if (cached !== null && parent === cachedParent) return cached;

    cachedParent = parent;
    cached = findScrollableAncestor(el);
    return cached;
  };
}
