import { defineStore } from 'pinia';
import { ref } from 'vue';

export interface NoteTab {
  slug: string;
  title: string;
}

const STORAGE_KEY = 'atlas:notes-tabs';

/**
 * Open notes tabs, keyed by workspace slug and persisted to localStorage so the
 * set of open notes survives a reload (Obsidian/VS Code style). The active tab is
 * not stored here — it is derived from the current route by the Notes view.
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

export const useNotesTabsStore = defineStore('notesTabs', () => {
  const byWorkspace = ref<Record<string, NoteTab[]>>(loadTabs());

  function persist(): void {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(byWorkspace.value));
    } catch {
      // ignore storage errors
    }
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

    return neighbour?.slug ?? null;
  }

  return { byWorkspace, tabs, open, setTitle, close };
});
