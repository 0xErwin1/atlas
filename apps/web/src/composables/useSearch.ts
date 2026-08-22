import Fuse from 'fuse.js';
import { useSearchStore } from '@/stores/search';

export type LocalActionKind = 'navigate' | 'action' | 'workspace';

interface LocalActionBase {
  id: string;
  label: string;
}

/**
 * A locally-resolved command shown in the command palette alongside server
 * search hits: a navigation, an app action, or a switch to another workspace
 * (which carries the destination slug). These are matched on the client with
 * fuse.js (Q6); server ranking remains authoritative for actual search hits.
 */
export type LocalAction =
  | (LocalActionBase & { kind: 'navigate' | 'action' })
  | (LocalActionBase & { kind: 'workspace'; slug: string });

/**
 * Builds one "switch to workspace" palette action per workspace the user can
 * reach, excluding the active one (switching to it would be a no-op).
 */
export function workspaceSwitchActions(
  workspaces: ReadonlyArray<{ slug: string; name: string }>,
  activeSlug: string | null,
): LocalAction[] {
  return workspaces
    .filter((workspace) => workspace.slug !== activeSlug)
    .map((workspace) => ({
      id: `switch-workspace:${workspace.slug}`,
      label: `Switch to workspace ${workspace.name}`,
      kind: 'workspace',
      slug: workspace.slug,
    }));
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
