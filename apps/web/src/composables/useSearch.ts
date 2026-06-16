import Fuse from 'fuse.js';
import { useSearchStore } from '@/stores/search';

export type LocalActionKind = 'navigate' | 'action';

/**
 * A locally-resolved command (navigation or action) shown in the command
 * palette alongside server search hits. These are matched on the client with
 * fuse.js (Q6); server ranking remains authoritative for actual search hits.
 */
export interface LocalAction {
  id: string;
  label: string;
  kind: LocalActionKind;
}

const FUSE_OPTIONS = {
  keys: ['label'],
  threshold: 0.4,
  ignoreLocation: true,
};

/**
 * Fuzzy-filter local navigation/actions by label using fuse.js.
 * An empty query returns the full list unchanged (palette default state).
 */
export function filterLocalActions(actions: LocalAction[], query: string): LocalAction[] {
  const trimmed = query.trim();
  if (trimmed === '') {
    return actions;
  }

  const fuse = new Fuse(actions, FUSE_OPTIONS);
  return fuse.search(trimmed).map((r) => r.item);
}

const DEFAULT_DEBOUNCE_MS = 200;

/**
 * useSearch — wraps the search store with input debouncing so rapid keystrokes
 * collapse into a single fresh search per settled query. Each settled query is a
 * fresh search (cursor reset); pagination of an existing result set is driven by
 * the store's loadMore.
 */
export function useSearch(ws: string, debounceMs: number = DEFAULT_DEBOUNCE_MS) {
  const store = useSearchStore();
  let timer: ReturnType<typeof setTimeout> | null = null;

  function onQueryInput(value: string): void {
    store.setQuery(value);

    if (timer !== null) {
      clearTimeout(timer);
    }

    timer = setTimeout(() => {
      timer = null;
      void store.runSearch(ws);
    }, debounceMs);
  }

  function loadMore(): Promise<void> {
    return store.loadMore(ws);
  }

  return { store, onQueryInput, loadMore };
}
