import { defineStore } from 'pinia';
import { ref } from 'vue';

/** A tab either holds a document (id = slug) or a board (id = board id). */
export type TabKind = 'doc' | 'board';

export interface DocsTab {
  kind: TabKind;
  id: string;
  title: string;
}

/** The identity of a tab, independent of its title. */
export type TabRef = Pick<DocsTab, 'kind' | 'id'>;

const STORAGE_KEY = 'atlas:notes-tabs';
const ACTIVE_STORAGE_KEY = 'atlas:notes-active-tab';

/**
 * A collision-free key for a tab across kinds: a doc slug and a board id could
 * otherwise coincide. Used as the DOM/identity key by the TabStrip so its
 * select/close events map back to the right tab.
 */
export function tabKey(ref: TabRef): string {
  return `${ref.kind}:${ref.id}`;
}

export function parseTabKey(key: string): TabRef {
  const separator = key.indexOf(':');
  const kind = key.slice(0, separator) as TabKind;
  return { kind, id: key.slice(separator + 1) };
}

function sameRef(a: TabRef, b: TabRef): boolean {
  return a.kind === b.kind && a.id === b.id;
}

/**
 * Normalizes a persisted tab entry to the typed shape. Entries written before
 * boards became tabs carry `{ slug, title }` with no `kind`; they are documents.
 */
function migrateTab(entry: unknown): DocsTab | null {
  if (typeof entry !== 'object' || entry === null) return null;

  const record = entry as Record<string, unknown>;
  const title = typeof record.title === 'string' ? record.title : '';

  if (record.kind === 'board' || record.kind === 'doc') {
    const id = typeof record.id === 'string' ? record.id : null;
    return id === null ? null : { kind: record.kind, id, title };
  }

  const slug = typeof record.slug === 'string' ? record.slug : null;
  return slug === null ? null : { kind: 'doc', id: slug, title };
}

function loadTabs(): Record<string, DocsTab[]> {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw === null) return {};

    const parsed = JSON.parse(raw) as Record<string, unknown>;
    const result: Record<string, DocsTab[]> = {};
    for (const [ws, list] of Object.entries(parsed)) {
      if (!Array.isArray(list)) continue;
      result[ws] = list.map(migrateTab).filter((tab): tab is DocsTab => tab !== null);
    }
    return result;
  } catch {
    return {};
  }
}

/**
 * Normalizes a persisted active pointer. Entries written before boards became
 * tabs stored a bare document slug string; they map to a doc ref.
 */
function migrateActive(entry: unknown): TabRef | null {
  if (typeof entry === 'string') return { kind: 'doc', id: entry };

  if (typeof entry === 'object' && entry !== null) {
    const record = entry as Record<string, unknown>;
    const id = typeof record.id === 'string' ? record.id : null;
    if (id !== null && (record.kind === 'board' || record.kind === 'doc')) {
      return { kind: record.kind, id };
    }
  }

  return null;
}

function loadActive(): Record<string, TabRef> {
  try {
    const raw = localStorage.getItem(ACTIVE_STORAGE_KEY);
    if (raw === null) return {};

    const parsed = JSON.parse(raw) as Record<string, unknown>;
    const result: Record<string, TabRef> = {};
    for (const [ws, value] of Object.entries(parsed)) {
      const ref = migrateActive(value);
      if (ref !== null) result[ws] = ref;
    }
    return result;
  } catch {
    return {};
  }
}

/**
 * Open document/board tabs, keyed by workspace slug and persisted to localStorage
 * so the set of open tabs survives a reload (Obsidian/VS Code style). The active
 * tab is persisted per workspace in a sibling key so a cold start with no restored
 * URL (the desktop webview boots at `/n` with no slug) can re-open whatever was
 * active last, instead of falling back to an empty pane.
 */
export const useNotesTabsStore = defineStore('notesTabs', () => {
  const byWorkspace = ref<Record<string, DocsTab[]>>(loadTabs());
  const activeByWorkspace = ref<Record<string, TabRef>>(loadActive());

  // The document whose editor currently has unsaved edits, per workspace. Only
  // the open document can be dirty, so a single ref per workspace suffices. Not
  // persisted: a reload starts clean.
  const dirtyDocByWorkspace = ref<Record<string, string>>({});

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

  /** The last active tab in `ws`, or null when none is recorded. */
  function activeFor(ws: string): TabRef | null {
    return activeByWorkspace.value[ws] ?? null;
  }

  function setActive(ws: string, ref: TabRef): void {
    const current = activeByWorkspace.value[ws];
    if (current !== undefined && sameRef(current, ref)) return;
    activeByWorkspace.value[ws] = { kind: ref.kind, id: ref.id };
    persistActive();
  }

  function clearActive(ws: string): void {
    if (activeByWorkspace.value[ws] === undefined) return;
    delete activeByWorkspace.value[ws];
    persistActive();
  }

  /**
   * Keeps the recorded active tab pointing at a still-open tab after a mutation:
   * a no-op when it survives, otherwise moved to `fallback` (or cleared when
   * `fallback` is null). Guarantees `activeFor` never references a closed tab.
   */
  function reconcileActive(ws: string, fallback: TabRef | null): void {
    const active = activeByWorkspace.value[ws];
    if (active === undefined) return;
    if ((byWorkspace.value[ws] ?? []).some((t) => sameRef(t, active))) return;

    if (fallback === null) clearActive(ws);
    else setActive(ws, fallback);
  }

  function tabs(ws: string): DocsTab[] {
    return byWorkspace.value[ws] ?? [];
  }

  /** Adds the tab if missing, otherwise refreshes its title. */
  function open(ws: string, ref: TabRef, title: string): void {
    const list = byWorkspace.value[ws] ?? [];
    const existing = list.find((t) => sameRef(t, ref));

    if (existing) {
      if (title !== '') existing.title = title;
    } else {
      byWorkspace.value[ws] = [...list, { kind: ref.kind, id: ref.id, title: title === '' ? ref.id : title }];
    }

    persist();
  }

  function setTitle(ws: string, ref: TabRef, title: string): void {
    if (title === '') return;
    const existing = (byWorkspace.value[ws] ?? []).find((t) => sameRef(t, ref));
    if (existing && existing.title !== title) {
      existing.title = title;
      persist();
    }
  }

  /**
   * Removes a tab and returns the tab that should become active when the closed
   * tab was the active one (the neighbour to its right, else its left), or null
   * when no tabs remain.
   */
  function close(ws: string, ref: TabRef): TabRef | null {
    const list = byWorkspace.value[ws] ?? [];
    const idx = list.findIndex((t) => sameRef(t, ref));
    if (idx === -1) return null;

    const neighbour = list[idx + 1] ?? list[idx - 1] ?? null;
    const neighbourRef = neighbour === null ? null : { kind: neighbour.kind, id: neighbour.id };

    byWorkspace.value[ws] = list.filter((t) => !sameRef(t, ref));
    persist();
    reconcileActive(ws, neighbourRef);

    return neighbourRef;
  }

  /** Closes every tab in the workspace. */
  function closeAll(ws: string): null {
    byWorkspace.value[ws] = [];
    persist();
    reconcileActive(ws, null);
    return null;
  }

  /** Closes every tab except `ref`. Returns the surviving ref (or null). */
  function closeOthers(ws: string, ref: TabRef): TabRef | null {
    const keep = (byWorkspace.value[ws] ?? []).find((t) => sameRef(t, ref));
    if (keep === undefined) return null;
    byWorkspace.value[ws] = [keep];
    persist();
    reconcileActive(ws, ref);
    return { kind: keep.kind, id: keep.id };
  }

  /** Closes every tab to the right of `ref`. Returns `ref` (or null if absent). */
  function closeRight(ws: string, ref: TabRef): TabRef | null {
    const list = byWorkspace.value[ws] ?? [];
    const idx = list.findIndex((t) => sameRef(t, ref));
    if (idx === -1) return null;
    byWorkspace.value[ws] = list.slice(0, idx + 1);
    persist();
    reconcileActive(ws, ref);
    return { kind: ref.kind, id: ref.id };
  }

  /** Records (or clears) which document tab currently has unsaved edits. */
  function setDirtyDoc(ws: string, id: string | null): void {
    if (id === null) {
      if (dirtyDocByWorkspace.value[ws] === undefined) return;
      delete dirtyDocByWorkspace.value[ws];
      return;
    }
    dirtyDocByWorkspace.value[ws] = id;
  }

  function isDirtyDoc(ws: string, id: string): boolean {
    return dirtyDocByWorkspace.value[ws] === id;
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
    clearActive,
    setDirtyDoc,
    isDirtyDoc,
  };
});
