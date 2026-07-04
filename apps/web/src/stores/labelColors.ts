import { defineStore } from 'pinia';
import { computed, ref } from 'vue';
import { defaultSwatchId } from '@/lib/swatches';

/**
 * User-chosen colors for labels / statuses / tags, persisted client-side.
 *
 * The backend does not (yet) carry a color on labels, columns or frontmatter
 * values, so the choice lives here in localStorage keyed by a stable identifier
 * (`tag:<name>`, `status:<value>`, `column:<id>`, …). Colors are explicitly
 * selected by the user — never derived from the value's text — and fall back to
 * a deterministic per-key default until a choice is made. When the API gains a
 * real color field this store becomes the migration source.
 */

export const STORAGE_KEY = 'atlas:label-colors';
export const TAGS_KEY = 'atlas:known-tags';

function load(): Record<string, string> {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw !== null) return JSON.parse(raw) as Record<string, string>;
  } catch {
    // ignore malformed storage
  }
  return {};
}

function loadTags(): string[] {
  try {
    const raw = localStorage.getItem(TAGS_KEY);
    if (raw !== null) return JSON.parse(raw) as string[];
  } catch {
    // ignore malformed storage
  }
  return [];
}

export const useLabelColorsStore = defineStore('labelColors', () => {
  const colors = ref<Record<string, string>>(load());

  // Tag names the app has actually seen (from real task labels), accumulated so
  // facets like search can offer them. There is no tags endpoint, so this is the
  // only real source; it grows as boards/tasks load and persists across sessions.
  const knownTags = ref<string[]>(loadTags());

  /** The chosen swatch id for a key, or its deterministic default. */
  function colorFor(key: string): string {
    return colors.value[key] ?? defaultSwatchId(key);
  }

  /** Records tag names seen in loaded data; deduped (case-insensitive) and sorted. */
  function recordTags(names: string[]): void {
    const byLower = new Map(knownTags.value.map((t) => [t.toLowerCase(), t]));
    let changed = false;

    for (const name of names) {
      const trimmed = name.trim();
      if (trimmed === '') continue;
      const key = trimmed.toLowerCase();
      if (!byLower.has(key)) {
        byLower.set(key, trimmed);
        changed = true;
      }
    }

    if (!changed) return;

    knownTags.value = [...byLower.values()].sort((a, b) => a.localeCompare(b));
    try {
      localStorage.setItem(TAGS_KEY, JSON.stringify(knownTags.value));
    } catch {
      // ignore storage errors
    }
  }

  const tagNames = computed(() => knownTags.value);

  /** Whether the user has explicitly picked a color for this key. */
  function isExplicit(key: string): boolean {
    return colors.value[key] !== undefined;
  }

  function setColor(key: string, swatchId: string): void {
    colors.value = { ...colors.value, [key]: swatchId };
    persist();
  }

  function persist(): void {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(colors.value));
    } catch {
      // ignore storage errors
    }
  }

  // Adopts a color map written by another tab (from a `storage` event). Parses
  // the raw payload defensively and never re-persists, so it cannot bounce a
  // fresh `storage` event back to the originating tab.
  function applyExternalColors(raw: string): void {
    try {
      const next = JSON.parse(raw) as unknown;
      if (next !== null && typeof next === 'object' && !Array.isArray(next)) {
        colors.value = next as Record<string, string>;
      }
    } catch {
      // ignore malformed payload
    }
  }

  // Adopts the known-tags list written by another tab, without re-persisting.
  function applyExternalKnownTags(raw: string): void {
    try {
      const next = JSON.parse(raw) as unknown;
      if (Array.isArray(next)) {
        knownTags.value = next as string[];
      }
    } catch {
      // ignore malformed payload
    }
  }

  return {
    colors,
    knownTags,
    tagNames,
    colorFor,
    isExplicit,
    setColor,
    recordTags,
    applyExternalColors,
    applyExternalKnownTags,
  };
});
