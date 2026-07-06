import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import { errorHint } from '@/lib/apiError';
import { collectPaged } from '@/lib/pagination';

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
  let loadSeq = 0;
  let loadingSeq = 0;

  /**
   * Load the active project's folders.
   *
   * A project/workspace switch (`silent: false`, the default) clears the list and
   * flips `loading` so the tree shows a loader instead of the previous project's
   * folders. A post-mutation refresh (`silent: true`) keeps the current tree in
   * place and updates it when the response arrives, so a rename/move/create never
   * blanks the whole tree. Both paths share the `loadSeq` guard so a slower
   * response can never overwrite a newer one.
   */
  async function load(ws: string, projectSlug: string, opts: { silent?: boolean } = {}): Promise<void> {
    const seq = ++loadSeq;
    const silent = opts.silent ?? false;

    error.value = null;
    if (!silent) {
      loadingSeq = seq;
      loading.value = true;
      folders.value = [];
    }

    const { items, error: apiError } = await collectPaged<FolderDto>((cursor) =>
      wrappedClient.GET('/api/workspaces/{ws}/projects/{project_slug}/folders', {
        params: {
          path: { ws, project_slug: projectSlug },
          query: { limit: 200, ...(cursor !== undefined ? { cursor } : {}) },
        },
      }),
    );

    if (seq !== loadSeq) {
      // A newer load supersedes this one. If we still own the loader (i.e. the
      // successor was a silent refresh, which never manages `loading`), release
      // it so the tree can never stay stuck on the spinner.
      if (!silent && seq === loadingSeq) loading.value = false;
      return;
    }

    if (!silent) loading.value = false;

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to load folders');
      return;
    }

    folders.value = items;
  }

  async function create(
    ws: string,
    projectSlug: string,
    name: string,
    parentFolderId?: string,
  ): Promise<boolean> {
    const { error: apiError } = await wrappedClient.POST(
      '/api/workspaces/{ws}/projects/{project_slug}/folders',
      {
        params: { path: { ws, project_slug: projectSlug } },
        body: { name, parent_folder_id: parentFolderId ?? null },
      },
    );

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to create folder');
      return false;
    }

    await load(ws, projectSlug, { silent: true });
    return true;
  }

  async function rename(ws: string, projectSlug: string, folderId: string, name: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.PATCH('/api/workspaces/{ws}/folders/{folder_id}', {
      params: { path: { ws, folder_id: folderId } },
      body: { name },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to rename folder');
      return false;
    }

    await load(ws, projectSlug, { silent: true });
    return true;
  }

  async function remove(ws: string, projectSlug: string, folderId: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.DELETE('/api/workspaces/{ws}/folders/{folder_id}', {
      params: { path: { ws, folder_id: folderId } },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to delete folder');
      return false;
    }

    await load(ws, projectSlug, { silent: true });
    return true;
  }

  async function move(
    ws: string,
    projectSlug: string,
    folderId: string,
    parentFolderId: string | null,
  ): Promise<boolean> {
    const { error: apiError } = await wrappedClient.PATCH('/api/workspaces/{ws}/folders/{folder_id}/move', {
      params: { path: { ws, folder_id: folderId } },
      body: { parent_folder_id: parentFolderId },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to move folder');
      return false;
    }

    await load(ws, projectSlug, { silent: true });
    return true;
  }

  async function copy(
    ws: string,
    projectSlug: string,
    folderId: string,
    parentFolderId: string | null,
  ): Promise<boolean> {
    const { error: apiError } = await wrappedClient.POST('/api/workspaces/{ws}/folders/{folder_id}/copy', {
      params: { path: { ws, folder_id: folderId } },
      body: { parent_folder_id: parentFolderId },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to copy folder');
      return false;
    }

    await load(ws, projectSlug, { silent: true });
    return true;
  }

  return { folders, loading, error, load, create, rename, remove, move, copy };
});
