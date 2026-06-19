import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';

export type FolderDto = components['schemas']['FolderDto'];

/**
 * Folders store: the single caller of the folder routes for the Notes tree
 * (REQ-W14). Holds the folders of the active project; mutations re-fetch the
 * project list so the tree stays consistent with the server.
 */
export const useFoldersStore = defineStore('folders', () => {
  const folders = ref<FolderDto[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);

  async function load(ws: string, projectSlug: string): Promise<void> {
    loading.value = true;
    error.value = null;

    const { data, error: apiError } = await wrappedClient.GET(
      '/v1/workspaces/{ws}/projects/{project_slug}/folders',
      { params: { path: { ws, project_slug: projectSlug } } },
    );

    loading.value = false;

    if (apiError !== undefined || data === undefined) {
      error.value =
        (apiError as { hint?: string; title?: string } | undefined)?.hint ?? 'Failed to load folders';
      return;
    }

    folders.value = data.items;
  }

  async function create(
    ws: string,
    projectSlug: string,
    name: string,
    parentFolderId?: string,
  ): Promise<boolean> {
    const { error: apiError } = await wrappedClient.POST(
      '/v1/workspaces/{ws}/projects/{project_slug}/folders',
      {
        params: { path: { ws, project_slug: projectSlug } },
        body: { name, parent_folder_id: parentFolderId ?? null },
      },
    );

    if (apiError !== undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to create folder';
      return false;
    }

    await load(ws, projectSlug);
    return true;
  }

  async function rename(ws: string, projectSlug: string, folderId: string, name: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.PATCH('/v1/workspaces/{ws}/folders/{folder_id}', {
      params: { path: { ws, folder_id: folderId } },
      body: { name },
    });

    if (apiError !== undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to rename folder';
      return false;
    }

    await load(ws, projectSlug);
    return true;
  }

  async function remove(ws: string, projectSlug: string, folderId: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.DELETE('/v1/workspaces/{ws}/folders/{folder_id}', {
      params: { path: { ws, folder_id: folderId } },
    });

    if (apiError !== undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to delete folder';
      return false;
    }

    await load(ws, projectSlug);
    return true;
  }

  async function move(
    ws: string,
    projectSlug: string,
    folderId: string,
    parentFolderId: string | null,
  ): Promise<boolean> {
    const { error: apiError } = await wrappedClient.PATCH('/v1/workspaces/{ws}/folders/{folder_id}/move', {
      params: { path: { ws, folder_id: folderId } },
      body: { parent_folder_id: parentFolderId },
    });

    if (apiError !== undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to move folder';
      return false;
    }

    await load(ws, projectSlug);
    return true;
  }

  async function copy(
    ws: string,
    projectSlug: string,
    folderId: string,
    parentFolderId: string | null,
  ): Promise<boolean> {
    const { error: apiError } = await wrappedClient.POST('/v1/workspaces/{ws}/folders/{folder_id}/copy', {
      params: { path: { ws, folder_id: folderId } },
      body: { parent_folder_id: parentFolderId },
    });

    if (apiError !== undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to copy folder';
      return false;
    }

    await load(ws, projectSlug);
    return true;
  }

  return { folders, loading, error, load, create, rename, remove, move, copy };
});
