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
  const foldersByProject = ref<Record<string, FolderDto[]>>({});
  const loading = ref(false);
  const loadingByProject = ref<Record<string, boolean>>({});
  const error = ref<string | null>(null);
  const loadSeqByProject = new Map<string, number>();
  let displaySeq = 0;
  let loadingDisplaySeq = 0;
  let displayProjectSlug: string | null = null;

  function foldersFor(projectSlug: string): FolderDto[] {
    return foldersByProject.value[projectSlug] ?? [];
  }

  function isProjectLoading(projectSlug: string): boolean {
    return loadingByProject.value[projectSlug] ?? false;
  }

  function clearProjectBuckets(): void {
    loadSeqByProject.clear();
    displaySeq += 1;
    loadingDisplaySeq = displaySeq;
    displayProjectSlug = null;
    folders.value = [];
    foldersByProject.value = {};
    loading.value = false;
    loadingByProject.value = {};
    error.value = null;
  }

  function publishForProject(projectSlug: string, items: FolderDto[]): void {
    foldersByProject.value = { ...foldersByProject.value, [projectSlug]: items };
    if (displayProjectSlug === null || displayProjectSlug === projectSlug) folders.value = items;
  }

  async function load(ws: string, projectSlug: string, opts: { silent?: boolean } = {}): Promise<void> {
    const seq = (loadSeqByProject.get(projectSlug) ?? 0) + 1;
    loadSeqByProject.set(projectSlug, seq);
    const currentDisplaySeq = ++displaySeq;
    const silent = opts.silent ?? false;

    error.value = null;
    if (!silent) {
      displayProjectSlug = projectSlug;
      loadingDisplaySeq = currentDisplaySeq;
      loading.value = true;
      loadingByProject.value = { ...loadingByProject.value, [projectSlug]: true };
      folders.value = [];
      foldersByProject.value = { ...foldersByProject.value, [projectSlug]: [] };
    }

    const { items, error: apiError } = await collectPaged<FolderDto>((cursor) =>
      wrappedClient.GET('/api/workspaces/{ws}/projects/{project_slug}/folders', {
        params: {
          path: { ws, project_slug: projectSlug },
          query: { limit: 200, ...(cursor !== undefined ? { cursor } : {}) },
        },
      }),
    );

    if (seq !== loadSeqByProject.get(projectSlug)) {
      if (!silent) {
        loadingByProject.value = { ...loadingByProject.value, [projectSlug]: false };
        if (currentDisplaySeq === loadingDisplaySeq) loading.value = false;
      }
      return;
    }

    if (!silent) {
      loadingByProject.value = { ...loadingByProject.value, [projectSlug]: false };
      if (currentDisplaySeq === loadingDisplaySeq) loading.value = false;
    }

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to load folders');
      return;
    }

    publishForProject(projectSlug, items);
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

  return {
    folders,
    foldersByProject,
    loading,
    error,
    foldersFor,
    isProjectLoading,
    clearProjectBuckets,
    publishForProject,
    load,
    create,
    rename,
    remove,
    move,
    copy,
  };
});
