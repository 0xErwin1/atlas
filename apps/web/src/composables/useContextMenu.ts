import { ref } from 'vue';

/**
 * Shared right-click / kebab context-menu state for sidebars. Holds the open flag
 * and the viewport position to render at, and opens at a mouse event's location.
 * Used by every sidebar tree (notes, tasks) so the menu behaviour is identical.
 */
export function useContextMenu() {
  const open = ref(false);
  const x = ref(0);
  const y = ref(0);

  function openAt(event: MouseEvent): void {
    x.value = event.clientX;
    y.value = event.clientY;
    open.value = true;
  }

  function close(): void {
    open.value = false;
  }

  return { open, x, y, openAt, close };
}
