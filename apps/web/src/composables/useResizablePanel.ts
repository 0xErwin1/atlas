import { ref } from 'vue';

interface ResizablePanelOptions {
  /** localStorage key the chosen width persists under. */
  storageKey: string;
  min: number;
  max: number;
  /** Width used when nothing valid is stored yet. */
  initial: number;
}

/**
 * Drag-to-resize state for a right-docked panel. The handle sits on the panel's
 * left edge, so dragging left widens it; the width is clamped to [min, max] and
 * persisted to localStorage on release. Returns the reactive width plus the
 * mousedown handler to wire onto the resize handle.
 */
export function useResizablePanel(options: ResizablePanelOptions) {
  const { storageKey, min, max, initial } = options;

  function clamp(value: number): number {
    return Math.min(Math.max(value, min), max);
  }

  function load(): number {
    try {
      const raw = localStorage.getItem(storageKey);
      const parsed = raw !== null ? Number.parseInt(raw, 10) : Number.NaN;
      if (Number.isFinite(parsed)) return clamp(parsed);
    } catch {
      // ignore storage errors
    }
    return initial;
  }

  const width = ref(load());

  let startX = 0;
  let startWidth = 0;

  function onMove(event: MouseEvent): void {
    // Right-docked panel: dragging the left-edge handle left widens it.
    const delta = startX - event.clientX;
    width.value = clamp(startWidth + delta);
  }

  function onEnd(): void {
    window.removeEventListener('mousemove', onMove);
    window.removeEventListener('mouseup', onEnd);
    document.body.style.removeProperty('cursor');
    document.body.style.removeProperty('user-select');
    try {
      localStorage.setItem(storageKey, String(width.value));
    } catch {
      // ignore storage errors
    }
  }

  function startResize(event: MouseEvent): void {
    startX = event.clientX;
    startWidth = width.value;
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onEnd);
  }

  return { width, startResize };
}
