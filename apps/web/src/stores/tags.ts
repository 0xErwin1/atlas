import { defineStore } from 'pinia';
import { computed, ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';

export type TagDto = components['schemas']['TagDto'];

/**
 * Workspace tag registry: the shared, server-backed pool of tag names used to
 * autocomplete and create tags across tasks and notes. Tags themselves stay
 * free-form strings on each task/note; this registry is what makes "select an
 * existing tag" possible and persists newly created ones for everyone.
 */
export const useTagsStore = defineStore('tags', () => {
  const tags = ref<TagDto[]>([]);
  const error = ref<string | null>(null);
  let loadedWs: string | null = null;

  const names = computed(() => tags.value.map((t) => t.name));

  async function load(ws: string, force = false): Promise<void> {
    if (!force && loadedWs === ws) return;

    const { data, error: apiError } = await wrappedClient.GET('/v1/workspaces/{ws}/tags', {
      params: { path: { ws } },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to load tags';
      return;
    }

    tags.value = data;
    loadedWs = ws;
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

    const { data, error: apiError } = await wrappedClient.POST('/v1/workspaces/{ws}/tags', {
      params: { path: { ws } },
      body: { name: trimmed },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to create tag';
      return null;
    }

    if (!tags.value.some((t) => t.id === data.id)) {
      tags.value = [...tags.value, data].sort((a, b) => a.name.localeCompare(b.name));
    }

    return data;
  }

  return { tags, names, error, load, ensure };
});
