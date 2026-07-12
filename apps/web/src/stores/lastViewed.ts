import { defineStore } from 'pinia';
import { ref } from 'vue';

export interface LastViewedTarget {
  name: string;
  params: Record<string, string>;
}

const STORAGE_KEY = 'atlas:last-viewed';

/** Reads the persisted map from localStorage, returning {} for missing or malformed data. */
function loadTargets(): Record<string, LastViewedTarget> {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as unknown;
      if (typeof parsed === 'object' && parsed !== null && !Array.isArray(parsed)) {
        return parsed as Record<string, LastViewedTarget>;
      }
    }
  } catch {
    // ignore malformed storage
  }
  return {};
}

function sameTarget(a: LastViewedTarget, b: LastViewedTarget): boolean {
  if (a.name !== b.name) return false;

  const aKeys = Object.keys(a.params);
  const bKeys = Object.keys(b.params);
  if (aKeys.length !== bKeys.length) return false;

  return aKeys.every((key) => a.params[key] === b.params[key]);
}

/**
 * The last resource-carrying route the user viewed in each workspace, keyed by
 * workspace slug and persisted to localStorage. On a workspace switch the rail
 * restores this target instead of showing a stale "not found" for a resource
 * that belongs to the previous workspace. Only targets that identify a concrete
 * resource are stored — a bare section root is the empty fallback and must never
 * overwrite a real entry.
 */
export const useLastViewedStore = defineStore('lastViewed', () => {
  const byWorkspace = ref<Record<string, LastViewedTarget>>(loadTargets());

  function persist(): void {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(byWorkspace.value));
    } catch {
      // ignore storage errors
    }
  }

  /** Upserts the last-viewed target for a workspace. */
  function record(ws: string, target: LastViewedTarget): void {
    byWorkspace.value[ws] = target;
    persist();
  }

  function forWorkspace(ws: string): LastViewedTarget | null {
    return byWorkspace.value[ws] ?? null;
  }

  /** Drops the entry so a resource that is gone stops being restored. */
  function clear(ws: string): void {
    if (byWorkspace.value[ws] === undefined) return;
    delete byWorkspace.value[ws];
    persist();
  }

  /**
   * Moves a workspace's entry to a new key, keeping the last-viewed resource
   * restorable after the workspace is re-slugged (entries are keyed by slug).
   * A no-op when the source has no entry or the key is unchanged. If the target
   * key already holds an entry it is left untouched (never clobbered) and the
   * source entry is not moved.
   */
  function rekey(oldWs: string, newWs: string): void {
    if (oldWs === newWs) return;

    const stored = byWorkspace.value[oldWs];
    if (stored === undefined) return;

    if (byWorkspace.value[newWs] !== undefined) return;

    delete byWorkspace.value[oldWs];
    byWorkspace.value[newWs] = stored;
    persist();
  }

  /**
   * Drops the entry only when it still points at `target`. Guards against a 404
   * on some other resource wiping a valid stored pointer for the workspace.
   */
  function clearIfMatches(ws: string, target: LastViewedTarget): void {
    const stored = byWorkspace.value[ws];
    if (stored === undefined || !sameTarget(stored, target)) return;
    delete byWorkspace.value[ws];
    persist();
  }

  return { byWorkspace, record, forWorkspace, clear, clearIfMatches, rekey };
});
