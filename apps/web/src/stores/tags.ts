import { defineStore } from 'pinia';
import { computed, ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import { errorHint } from '@/lib/apiError';
import { defaultSwatchId } from '@/lib/swatches';

export type TagDto = components['schemas']['TagDto'];

/**
 * Workspace tag registry: the shared, server-backed pool of tag names used to
 * autocomplete and create tags across tasks and notes. Tags themselves stay
 * free-form strings on each task/note; this registry is what makes "select an
 * existing tag" possible and persists newly created ones for everyone.
 */
export const useTagsStore = defineStore('tags', () => {
  const tags = ref<TagDto[]>([]);
  const usedLabels = ref<string[]>([]);
  const error = ref<string | null>(null);
  let loadedWs: string | null = null;
  let loadedUsedWs: string | null = null;

  const names = computed(() => tags.value.map((t) => t.name));

  const byLowerName = computed(() => new Map(tags.value.map((t) => [t.name.toLowerCase(), t])));

  /**
   * Labels actually used on tasks that are NOT in the registry, compared
   * case-insensitively so a registered tag never resurfaces here. These are the
   * usage-derived labels a user can promote into the managed registry.
   */
  const unregisteredLabels = computed(() =>
    usedLabels.value.filter((label) => !byLowerName.value.has(label.toLowerCase())),
  );

  /**
   * The swatch id for a tag name. The backend color is the source of truth; a
   * tag with no explicit color (or one not in the registry) falls back to the
   * deterministic per-name default — the same default used elsewhere, so colors
   * stay consistent until a user recolors the tag in settings.
   */
  function colorFor(name: string): string {
    const lower = name.toLowerCase();
    return byLowerName.value.get(lower)?.color ?? defaultSwatchId(`tag:${lower}`);
  }

  async function load(ws: string, force = false): Promise<void> {
    if (!force && loadedWs === ws) return;

    const { data, error: apiError } = await wrappedClient.GET('/api/workspaces/{ws}/tags', {
      params: { path: { ws } },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to load tags');
      return;
    }

    tags.value = data;
    loadedWs = ws;
  }

  /**
   * Loads the distinct labels actually used on the workspace's tasks. The
   * registry merge (see `unregisteredLabels`) is done client-side, so this only
   * needs the raw usage list.
   */
  async function loadUsed(ws: string, force = false): Promise<void> {
    if (!force && loadedUsedWs === ws) return;

    const { data, error: apiError } = await wrappedClient.GET('/api/workspaces/{ws}/tags/used', {
      params: { path: { ws } },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to load used labels');
      return;
    }

    usedLabels.value = data;
    loadedUsedWs = ws;
  }

  /**
   * Ensures a tag exists in the registry (the endpoint is idempotent by
   * case-insensitive name) and caches it locally. Returns the canonical tag, or
   * null on failure.
   */
  async function ensure(ws: string, name: string): Promise<TagDto | null> {
    const trimmed = name.trim();
    if (trimmed === '') return null;

    const existing = tags.value.find((t) => t.name.toLowerCase() === trimmed.toLowerCase());
    if (existing !== undefined) return existing;

    const { data, error: apiError } = await wrappedClient.POST('/api/workspaces/{ws}/tags', {
      params: { path: { ws } },
      body: { name: trimmed },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to create tag');
      return null;
    }

    if (!tags.value.some((t) => t.id === data.id)) {
      tags.value = [...tags.value, data].sort((a, b) => a.name.localeCompare(b.name));
    }

    return data;
  }

  /**
   * Creates a tag with an explicit name and optional color, caching it locally.
   * Unlike `ensure` this always POSTs (the endpoint is idempotent by name, so a
   * duplicate name returns the existing tag). Returns the tag, or null on failure.
   */
  async function create(ws: string, name: string, color?: string | null): Promise<TagDto | null> {
    const trimmed = name.trim();
    if (trimmed === '') return null;

    const body: { name: string; color?: string | null } = { name: trimmed };
    if (color !== undefined) body.color = color;

    const { data, error: apiError } = await wrappedClient.POST('/api/workspaces/{ws}/tags', {
      params: { path: { ws } },
      body,
    });

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to create tag');
      return null;
    }

    if (!tags.value.some((t) => t.id === data.id)) {
      tags.value = [...tags.value, data].sort((a, b) => a.name.localeCompare(b.name));
    }

    return data;
  }

  /**
   * Renames and/or recolors a tag. A rename backfills task labels across the
   * workspace server-side. Replaces the cached tag with the returned canonical
   * one (re-sorted by name). Returns true on success; sets `error` otherwise.
   */
  async function update(
    ws: string,
    id: string,
    patch: { name?: string; color?: string | null },
  ): Promise<boolean> {
    const { data, error: apiError } = await wrappedClient.PATCH('/api/workspaces/{ws}/tags/{tag_id}', {
      params: { path: { ws, tag_id: id } },
      body: patch,
    });

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to update tag');
      return false;
    }

    tags.value = tags.value.map((t) => (t.id === id ? data : t)).sort((a, b) => a.name.localeCompare(b.name));
    return true;
  }

  /** Deletes a tag and drops it from the cache. Returns true on success. */
  async function remove(ws: string, id: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.DELETE('/api/workspaces/{ws}/tags/{tag_id}', {
      params: { path: { ws, tag_id: id } },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to delete tag');
      return false;
    }

    tags.value = tags.value.filter((t) => t.id !== id);
    return true;
  }

  return {
    tags,
    usedLabels,
    unregisteredLabels,
    names,
    error,
    load,
    loadUsed,
    ensure,
    create,
    update,
    remove,
    colorFor,
  };
});
