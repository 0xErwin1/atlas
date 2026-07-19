import { defineStore } from 'pinia';
import { ref } from 'vue';

export interface NoteTab {
  slug: string;
  title: string;
}

const STORAGE_KEY = 'atlas:notes-tabs';
const ACTIVE_STORAGE_KEY = 'atlas:notes-active-tab';

/**
 * Open notes tabs, keyed by workspace slug and persisted to localStorage so the
 * set of open notes survives a reload (Obsidian/VS Code style). The active tab is
 * persisted per workspace in a sibling key so a cold start with no restored URL
 * (the desktop webview boots at `/n` with no slug) can re-open the note that was
 * active last, instead of falling back to an empty pane.
 */
function loadTabs(): Record<string, NoteTab[]> {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) return JSON.parse(raw) as Record<string, NoteTab[]>;
  } catch {
    // ignore malformed storage
  }
  return {};
}

function loadActive(): Record<string, string> {
  try {
    const raw = localStorage.getItem(ACTIVE_STORAGE_KEY);
    if (raw) return JSON.parse(raw) as Record<string, string>;
  } catch {
    // ignore malformed storage
  }
  return {};
}

export const useNotesTabsStore = defineStore('notesTabs', () => {
  const byWorkspace = ref<Record<string, NoteTab[]>>(loadTabs());
  const activeByWorkspace = ref<Record<string, string>>(loadActive());

  function persist(): void {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(byWorkspace.value));
    } catch {
      // ignore storage errors
    }
  }

  function persistActive(): void {
    try {
      localStorage.setItem(ACTIVE_STORAGE_KEY, JSON.stringify(activeByWorkspace.value));
    } catch {
      // ignore storage errors
    }
  }

  /** The slug of the last active tab in `ws`, or null when none is recorded. */
  function activeFor(ws: string): string | null {
    return activeByWorkspace.value[ws] ?? null;
  }

  function setActive(ws: string, slug: string): void {
    if (activeByWorkspace.value[ws] === slug) return;
    activeByWorkspace.value[ws] = slug;
    persistActive();
  }

  function clearActive(ws: string): void {
    if (activeByWorkspace.value[ws] === undefined) return;
    delete activeByWorkspace.value[ws];
    persistActive();
  }

  /**
   * Keeps the recorded active slug pointing at a still-open tab after a mutation:
   * a no-op when it survives, otherwise moved to `fallback` (or cleared when
   * `fallback` is null). Guarantees `activeFor` never references a closed tab.
   */
  function reconcileActive(ws: string, fallback: string | null): void {
    const active = activeByWorkspace.value[ws];
    if (active === undefined) return;
    if ((byWorkspace.value[ws] ?? []).some((t) => t.slug === active)) return;

    if (fallback === null) clearActive(ws);
    else setActive(ws, fallback);
  }

  function tabs(ws: string): NoteTab[] {
    return byWorkspace.value[ws] ?? [];
  }

  /** Adds the tab if missing, otherwise refreshes its title. */
  function open(ws: string, slug: string, title: string): void {
    const list = byWorkspace.value[ws] ?? [];
    const existing = list.find((t) => t.slug === slug);

    if (existing) {
      if (title !== '') existing.title = title;
    } else {
      byWorkspace.value[ws] = [...list, { slug, title: title === '' ? slug : title }];
    }

    persist();
  }

  function setTitle(ws: string, slug: string, title: string): void {
    if (title === '') return;
    const existing = (byWorkspace.value[ws] ?? []).find((t) => t.slug === slug);
    if (existing && existing.title !== title) {
      existing.title = title;
      persist();
    }
  }

  /**
   * Removes a tab and returns the slug that should become active when the closed
   * tab was the active one (the neighbour to its right, else its left), or null
   * when no tabs remain.
   */
  function close(ws: string, slug: string): string | null {
    const list = byWorkspace.value[ws] ?? [];
    const idx = list.findIndex((t) => t.slug === slug);
    if (idx === -1) return null;

    const neighbour = list[idx + 1] ?? list[idx - 1] ?? null;
    byWorkspace.value[ws] = list.filter((t) => t.slug !== slug);
    persist();
    reconcileActive(ws, neighbour?.slug ?? null);

    return neighbour?.slug ?? null;
  }

  /** Closes every tab in the workspace. */
  function closeAll(ws: string): null {
    byWorkspace.value[ws] = [];
    persist();
    reconcileActive(ws, null);
    return null;
  }

  /** Closes every tab except `slug`. Returns the surviving slug (or null). */
  function closeOthers(ws: string, slug: string): string | null {
    const keep = (byWorkspace.value[ws] ?? []).find((t) => t.slug === slug);
    if (keep === undefined) return null;
    byWorkspace.value[ws] = [keep];
    persist();
    reconcileActive(ws, slug);
    return slug;
  }

  /** Closes every tab to the right of `slug`. Returns `slug` (or null if absent). */
  function closeRight(ws: string, slug: string): string | null {
    const list = byWorkspace.value[ws] ?? [];
    const idx = list.findIndex((t) => t.slug === slug);
    if (idx === -1) return null;
    byWorkspace.value[ws] = list.slice(0, idx + 1);
    persist();
    reconcileActive(ws, slug);
    return slug;
  }

  return {
    byWorkspace,
    tabs,
    open,
    setTitle,
    close,
    closeAll,
    closeOthers,
    closeRight,
    activeFor,
    setActive,
  };
});
