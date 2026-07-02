import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import { errorHint } from '@/lib/apiError';
import { collectPaged } from '@/lib/pagination';

export type DocumentSummary = components['schemas']['Page_DocumentSummaryDto']['items'][number];
export type BacklinkSummary = components['schemas']['Page_BacklinkDto']['items'][number];
export type CommentDto = components['schemas']['CommentDto'];

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
  const comments = ref<CommentDto[]>([]);
  const commentsCursor = ref<string | null>(null);
  const commentsHasMore = ref(false);
  const loading = ref(false);
  const error = ref<string | null>(null);
  let summariesLoadSeq = 0;
  let summariesLoadingSeq = 0;

  /**
   * Load the active project's document summaries.
   *
   * A project/workspace switch (`silent: false`, the default) clears the list and
   * flips `loading` so the tree shows a loader instead of the previous project's
   * notes. A post-mutation refresh (`silent: true`) keeps the current tree in
   * place and updates it when the response arrives, so a rename/move/create never
   * blanks the whole tree. Both paths share the `summariesLoadSeq` guard so a
   * slower response can never overwrite a newer one.
   */
  async function loadSummaries(
    ws: string,
    projectSlug: string,
    opts: { silent?: boolean } = {},
  ): Promise<void> {
    const seq = ++summariesLoadSeq;
    const silent = opts.silent ?? false;

    error.value = null;
    if (!silent) {
      summariesLoadingSeq = seq;
      loading.value = true;
      summaries.value = [];
    }

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

    if (seq !== summariesLoadSeq) {
      // A newer load supersedes this one. If we still own the loader (i.e. the
      // successor was a silent refresh, which never manages `loading`), release
      // it so the tree can never stay stuck on the spinner.
      if (!silent && seq === summariesLoadingSeq) loading.value = false;
      return;
    }

    if (!silent) loading.value = false;

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to load documents');
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

  /**
   * Loads the first page of a document's comments (oldest-first). Mirrors the
   * task-comment thread in the task inspector, but keyed by document slug.
   */
  async function loadComments(ws: string, slug: string): Promise<void> {
    error.value = null;

    const { data, error: apiError } = await wrappedClient.GET(
      '/v1/workspaces/{ws}/documents/{slug}/comments',
      { params: { path: { ws, slug } } },
    );

    if (apiError !== undefined || data === undefined) {
      comments.value = [];
      commentsCursor.value = null;
      commentsHasMore.value = false;
      error.value = errorHint(apiError, 'Failed to load comments');
      return;
    }

    comments.value = data.items;
    commentsCursor.value = data.next_cursor ?? null;
    commentsHasMore.value = data.has_more;
  }

  /** Appends the next page of comments using the stored cursor. No-op at the end. */
  async function loadMoreComments(ws: string, slug: string): Promise<void> {
    error.value = null;

    if (!commentsHasMore.value || commentsCursor.value === null) {
      return;
    }

    const { data, error: apiError } = await wrappedClient.GET(
      '/v1/workspaces/{ws}/documents/{slug}/comments',
      { params: { path: { ws, slug }, query: { cursor: commentsCursor.value } } },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to load comments');
      return;
    }

    comments.value = [...comments.value, ...data.items];
    commentsCursor.value = data.next_cursor ?? null;
    commentsHasMore.value = data.has_more;
  }

  async function addComment(ws: string, slug: string, body: string): Promise<boolean> {
    error.value = null;

    const { data, error: apiError } = await wrappedClient.POST(
      '/v1/workspaces/{ws}/documents/{slug}/comments',
      { params: { path: { ws, slug } }, body: { body } },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to add comment');
      return false;
    }

    // Only reflect locally once the whole thread is paged in, so a newest-first
    // append can't land out of order or be re-fetched as a duplicate by a later
    // "Load more". It is persisted server-side regardless.
    if (!commentsHasMore.value) {
      comments.value = [...comments.value, data];
    }
    return true;
  }

  async function removeComment(ws: string, slug: string, commentId: string): Promise<boolean> {
    error.value = null;

    const snapshot = [...comments.value];
    comments.value = comments.value.filter((c) => c.id !== commentId);

    const { error: apiError } = await wrappedClient.DELETE(
      '/v1/workspaces/{ws}/documents/{slug}/comments/{comment_id}',
      { params: { path: { ws, slug, comment_id: commentId } } },
    );

    if (apiError !== undefined) {
      comments.value = snapshot;
      error.value = errorHint(apiError, 'Failed to remove comment');
      return false;
    }

    return true;
  }

  /** Edits a comment's body (author-only server-side); swaps the DTO in place. */
  async function editComment(ws: string, slug: string, commentId: string, body: string): Promise<boolean> {
    error.value = null;

    const { data, error: apiError } = await wrappedClient.PATCH(
      '/v1/workspaces/{ws}/documents/{slug}/comments/{comment_id}',
      { params: { path: { ws, slug, comment_id: commentId } }, body: { body } },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to edit comment');
      return false;
    }

    const idx = comments.value.findIndex((c) => c.id === commentId);
    if (idx !== -1) {
      const updated = [...comments.value];
      updated[idx] = data;
      comments.value = updated;
    }

    return true;
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
      error.value = errorHint(apiError, 'Failed to create document');
      return null;
    }

    await loadSummaries(ws, projectSlug, { silent: true });
    return data.slug ?? null;
  }

  async function rename(ws: string, projectSlug: string, slug: string, title: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.PATCH('/v1/workspaces/{ws}/documents/{slug}', {
      params: { path: { ws, slug } },
      body: { title },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to rename document');
      return false;
    }

    await loadSummaries(ws, projectSlug, { silent: true });
    return true;
  }

  async function remove(ws: string, projectSlug: string, slug: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.DELETE('/v1/workspaces/{ws}/documents/{slug}', {
      params: { path: { ws, slug } },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to delete document');
      return false;
    }

    await loadSummaries(ws, projectSlug, { silent: true });
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
      error.value = errorHint(apiError, 'Failed to move document');
      return false;
    }

    await loadSummaries(ws, projectSlug, { silent: true });
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
      error.value = errorHint(apiError, 'Failed to copy document');
      return false;
    }

    await loadSummaries(ws, projectSlug, { silent: true });
    return true;
  }

  return {
    summaries,
    backlinks,
    comments,
    commentsHasMore,
    loading,
    error,
    loadSummaries,
    loadBacklinks,
    loadComments,
    loadMoreComments,
    addComment,
    removeComment,
    editComment,
    create,
    rename,
    remove,
    move,
    copy,
  };
});
