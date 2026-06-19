import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import { collectPaged } from '@/lib/pagination';

export type DocumentSummary = components['schemas']['Page_DocumentSummaryDto']['items'][number];
export type BacklinkSummary = components['schemas']['Page_BacklinkDto']['items'][number];

/**
 * Documents store: the single caller of the document list and backlink routes
 * for the Notes app. Holds the document summaries of the active project (used
 * by the tree, REQ-W14) and the backlinks of the open document (REQ-W17).
 *
 * Per the design (Q7) the editor body itself is loaded via useMarkdownDoc, not
 * cached here; this store keeps lightweight summaries and backlinks only.
 */
export const useDocumentsStore = defineStore('documents', () => {
  const summaries = ref<DocumentSummary[]>([]);
  const backlinks = ref<BacklinkSummary[]>([]);
  const loading = ref(false);
  const error = ref<string | null>(null);

  async function loadSummaries(ws: string, projectSlug: string): Promise<void> {
    loading.value = true;
    error.value = null;

    // The tree renders the whole project, but the endpoint is paginated. Page
    // through it so all documents show — and so a newly created note (newest by
    // UUIDv7, hence on the last page) is never dropped.
    const { items, error: apiError } = await collectPaged<DocumentSummary>((cursor) =>
      wrappedClient.GET('/v1/workspaces/{ws}/projects/{project_slug}/documents', {
        params: {
          path: { ws, project_slug: projectSlug },
          query: { limit: 200, ...(cursor !== undefined ? { cursor } : {}) },
        },
      }),
    );

    loading.value = false;

    if (apiError !== undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to load documents';
      return;
    }

    summaries.value = items;
  }

  async function loadBacklinks(ws: string, slug: string): Promise<void> {
    const { items, error: apiError } = await collectPaged<BacklinkSummary>((cursor) =>
      wrappedClient.GET('/v1/workspaces/{ws}/documents/{slug}/backlinks', {
        params: { path: { ws, slug }, query: { limit: 200, ...(cursor !== undefined ? { cursor } : {}) } },
      }),
    );

    if (apiError !== undefined) {
      backlinks.value = [];
      return;
    }

    backlinks.value = items;
  }

  async function create(
    ws: string,
    projectSlug: string,
    title: string,
    folderId?: string,
  ): Promise<string | null> {
    const { data, error: apiError } = await wrappedClient.POST(
      '/v1/workspaces/{ws}/projects/{project_slug}/documents',
      {
        params: { path: { ws, project_slug: projectSlug } },
        body: { title, folder_id: folderId ?? null },
      },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to create document';
      return null;
    }

    await loadSummaries(ws, projectSlug);
    return data.slug ?? null;
  }

  async function rename(ws: string, projectSlug: string, slug: string, title: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.PATCH('/v1/workspaces/{ws}/documents/{slug}', {
      params: { path: { ws, slug } },
      body: { title },
    });

    if (apiError !== undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to rename document';
      return false;
    }

    await loadSummaries(ws, projectSlug);
    return true;
  }

  async function remove(ws: string, projectSlug: string, slug: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.DELETE('/v1/workspaces/{ws}/documents/{slug}', {
      params: { path: { ws, slug } },
    });

    if (apiError !== undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to delete document';
      return false;
    }

    await loadSummaries(ws, projectSlug);
    return true;
  }

  async function move(
    ws: string,
    projectSlug: string,
    slug: string,
    folderId: string | null,
  ): Promise<boolean> {
    const { error: apiError } = await wrappedClient.PATCH('/v1/workspaces/{ws}/documents/{slug}/move', {
      params: { path: { ws, slug } },
      body: { folder_id: folderId },
    });

    if (apiError !== undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to move document';
      return false;
    }

    await loadSummaries(ws, projectSlug);
    return true;
  }

  async function copy(
    ws: string,
    projectSlug: string,
    slug: string,
    folderId: string | null,
  ): Promise<boolean> {
    const { error: apiError } = await wrappedClient.POST('/v1/workspaces/{ws}/documents/{slug}/copy', {
      params: { path: { ws, slug } },
      body: { folder_id: folderId },
    });

    if (apiError !== undefined) {
      error.value = (apiError as { hint?: string } | undefined)?.hint ?? 'Failed to copy document';
      return false;
    }

    await loadSummaries(ws, projectSlug);
    return true;
  }

  return {
    summaries,
    backlinks,
    loading,
    error,
    loadSummaries,
    loadBacklinks,
    create,
    rename,
    remove,
    move,
    copy,
  };
});
