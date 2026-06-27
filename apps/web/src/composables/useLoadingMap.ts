import { ref } from 'vue';

/**
 * Tracks per-item async state keyed by id, for lists where individual rows load
 * independently (expanding a row to fetch its detail). Replaces the inline
 * `Record<string, boolean>` plus spread-to-update boilerplate each panel repeated.
 */
export function useLoadingMap() {
  const map = ref<Record<string, boolean>>({});

  function set(id: string, value: boolean): void {
    map.value = { ...map.value, [id]: value };
  }

  function isLoading(id: string): boolean {
    return map.value[id] === true;
  }

  return { isLoading, set };
}
