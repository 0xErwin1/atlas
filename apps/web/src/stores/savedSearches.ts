import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';

export type SavedSearchDto = components['schemas']['SavedSearchDto'];

export const useSavedSearchesStore = defineStore('savedSearches', () => {
  const items = ref<SavedSearchDto[]>([]);
  const error = ref<string | null>(null);
  let loadedWs: string | null = null;

  async function load(ws: string, force = false): Promise<void> {
    if (!force && loadedWs === ws) return;

    const { data, error: apiError } = await wrappedClient.GET('/v1/workspaces/{ws}/saved-searches', {
      params: { path: { ws } },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to load saved searches';
      return;
    }

    items.value = data;
    loadedWs = ws;
  }

  async function create(
    ws: string,
    payload: { name: string; query: string },
  ): Promise<SavedSearchDto | null> {
    const { data, error: apiError } = await wrappedClient.POST('/v1/workspaces/{ws}/saved-searches', {
      params: { path: { ws } },
      body: { name: payload.name, query: payload.query },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to save search';
      return null;
    }

    items.value = [...items.value, data].sort((a, b) => a.name.localeCompare(b.name));
    return data;
  }

  async function rename(ws: string, id: string, name: string): Promise<boolean> {
    const { data, error: apiError } = await wrappedClient.PATCH('/v1/workspaces/{ws}/saved-searches/{id}', {
      params: { path: { ws, id } },
      body: { name },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to rename saved search';
      return false;
    }

    items.value = items.value
      .map((s) => (s.id === id ? data : s))
      .sort((a, b) => a.name.localeCompare(b.name));

    return true;
  }

  async function remove(ws: string, id: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.DELETE('/v1/workspaces/{ws}/saved-searches/{id}', {
      params: { path: { ws, id } },
    });

    if (apiError !== undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to delete saved search';
      return false;
    }

    items.value = items.value.filter((s) => s.id !== id);
    return true;
  }

  return { items, error, load, create, rename, remove };
});
