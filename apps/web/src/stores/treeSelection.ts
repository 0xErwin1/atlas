import { defineStore } from 'pinia';
import { computed, ref } from 'vue';

export interface ClickModifiers {
  shift?: boolean;
  meta?: boolean;
}

/**
 * Selection state for the notes sidebar tree, shared across the recursive rows.
 * Keys are the opaque node keys from `lib/notesTree` (`folder:<id>` / `doc:<slug>`).
 *
 * `order` is the flattened visible key list (set by the tree) and is what makes
 * shift-range selection possible without each row knowing the global layout.
 */
export const useTreeSelection = defineStore('treeSelection', () => {
  const selected = ref<Set<string>>(new Set());
  const anchor = ref<string | null>(null);
  const order = ref<string[]>([]);

  const count = computed(() => selected.value.size);

  function setOrder(keys: string[]): void {
    order.value = keys;
  }

  function isSelected(key: string): boolean {
    return selected.value.has(key);
  }

  function keys(): string[] {
    return [...selected.value];
  }

  function clear(): void {
    selected.value = new Set();
    anchor.value = null;
  }

  function selectOnly(key: string): void {
    selected.value = new Set([key]);
    anchor.value = key;
  }

  function toggle(key: string): void {
    const next = new Set(selected.value);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    selected.value = next;
    anchor.value = key;
  }

  function selectRange(key: string): void {
    const from = anchor.value;
    const i = from === null ? -1 : order.value.indexOf(from);
    const j = order.value.indexOf(key);
    if (i === -1 || j === -1) {
      selectOnly(key);
      return;
    }
    const [lo, hi] = i <= j ? [i, j] : [j, i];
    selected.value = new Set(order.value.slice(lo, hi + 1));
    // anchor is kept so successive shift-clicks extend from the same origin.
  }

  /**
   * Applies a row click with its modifier keys and reports whether the caller
   * should still perform its default action (open a doc / toggle a folder).
   * Shift and ctrl/cmd only mutate the selection; a plain click selects just
   * that row and lets the default action proceed.
   */
  function activate(key: string, mods: ClickModifiers): 'default' | 'selection-only' {
    if (mods.shift === true) {
      selectRange(key);
      return 'selection-only';
    }
    if (mods.meta === true) {
      toggle(key);
      return 'selection-only';
    }
    selectOnly(key);
    return 'default';
  }

  return {
    selected,
    anchor,
    order,
    count,
    setOrder,
    isSelected,
    keys,
    clear,
    selectOnly,
    toggle,
    selectRange,
    activate,
  };
});
