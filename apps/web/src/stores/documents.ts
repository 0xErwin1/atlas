import { defineStore } from 'pinia';
import { ref } from 'vue';
import { z } from 'zod';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import { getResourceCachePrincipal, resourceCache } from '@/cache/cacheRuntime';
import { buildCacheKey, CACHE_CADENCE } from '@/cache/resourceCache';
import { errorHint } from '@/lib/apiError';
import { attachmentFileName } from '@/lib/fileTransfer';
import { collectPaged } from '@/lib/pagination';

export type DocumentSummary = components['schemas']['Page_DocumentSummaryDto']['items'][number];
export type BacklinkSummary = components['schemas']['Page_BacklinkDto']['items'][number];
export type CommentDto = components['schemas']['CommentDto'];
export type AttachmentDto = components['schemas']['AttachmentDto'];
export type SecondaryLoadStatus = 'idle' | 'pending' | 'ready' | 'error';

type CommentListResponse = components['schemas']['CommentListResponseDto'];

type SecondaryTarget = {
  workspaceSlug: string;
  slug: string;
};

function legacyCommentItems(data: CommentListResponse): CommentDto[] | null {
  const legacyItems: CommentDto[] = [];

  for (const item of data.items) {
    if ('type' in item) return null;
    legacyItems.push(item);
  }

  return legacyItems;
}

type BacklinksCacheOptions = {
  workspaceId: string;
};

const backlinksSchema: z.ZodType<BacklinkSummary[]> = z.array(
  z
    .object({
      source_document_id: z.string(),
      source_slug: z.string().nullable().optional(),
      source_title: z.string().nullable().optional(),
      display_title: z.string().nullable().optional(),
    })
    .passthrough(),
) as z.ZodType<BacklinkSummary[]>;

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
  const summariesByProject = ref<Record<string, DocumentSummary[]>>({});
  const backlinks = ref<BacklinkSummary[]>([]);
  const backlinksStatus = ref<SecondaryLoadStatus>('idle');
  const backlinksError = ref<string | null>(null);
  const comments = ref<CommentDto[]>([]);
  const commentsCursor = ref<string | null>(null);
  const commentsHasMore = ref(false);
  const commentsStatus = ref<SecondaryLoadStatus>('idle');
  const commentsError = ref<string | null>(null);
  const loading = ref(false);
  const loadingByProject = ref<Record<string, boolean>>({});
  const error = ref<string | null>(null);
  const summariesLoadSeqByProject = new Map<string, number>();
  let summariesDisplaySeq = 0;
  let summariesLoadingDisplaySeq = 0;
  let summariesDisplayProjectSlug: string | null = null;
  let secondaryTarget: SecondaryTarget | null = null;
  let backlinksLoadSeq = 0;
  let commentsLoadSeq = 0;
  let commentsTargetEpoch = 0;
  let activeBacklinksCacheKey: string | null = null;

  function summariesFor(projectSlug: string): DocumentSummary[] {
    return summariesByProject.value[projectSlug] ?? [];
  }

  function isProjectLoading(projectSlug: string): boolean {
    return loadingByProject.value[projectSlug] ?? false;
  }

  function clearProjectBuckets(): void {
    summariesLoadSeqByProject.clear();
    summariesDisplaySeq += 1;
    summariesLoadingDisplaySeq = summariesDisplaySeq;
    summariesDisplayProjectSlug = null;
    summaries.value = [];
    summariesByProject.value = {};
    loading.value = false;
    loadingByProject.value = {};
    error.value = null;
    clearSecondaryTarget();
  }

  function publishSummariesForProject(projectSlug: string, items: DocumentSummary[]): void {
    summariesByProject.value = { ...summariesByProject.value, [projectSlug]: items };
    if (summariesDisplayProjectSlug === null || summariesDisplayProjectSlug === projectSlug) {
      summaries.value = items;
    }
  }

  function isSecondaryTarget(ws: string, slug: string): boolean {
    return secondaryTarget?.workspaceSlug === ws && secondaryTarget.slug === slug;
  }

  function isCurrentCommentsTarget(target: SecondaryTarget, generation: number): boolean {
    return generation === commentsTargetEpoch && isSecondaryTarget(target.workspaceSlug, target.slug);
  }

  function deactivateBacklinksCache(): void {
    if (activeBacklinksCacheKey === null) return;

    resourceCache.deactivate(activeBacklinksCacheKey);
    activeBacklinksCacheKey = null;
  }

  function clearSecondaryState(): void {
    deactivateBacklinksCache();
    backlinksLoadSeq += 1;
    commentsLoadSeq += 1;
    commentsTargetEpoch += 1;
    backlinks.value = [];
    backlinksStatus.value = 'idle';
    backlinksError.value = null;
    comments.value = [];
    commentsCursor.value = null;
    commentsHasMore.value = false;
    commentsStatus.value = 'idle';
    commentsError.value = null;
  }

  function resetSecondaryTarget(ws: string, slug: string): void {
    if (isSecondaryTarget(ws, slug)) return;

    secondaryTarget = { workspaceSlug: ws, slug };
    clearSecondaryState();
  }

  function clearSecondaryTarget(): void {
    if (secondaryTarget === null) return;

    secondaryTarget = null;
    clearSecondaryState();
  }

  async function loadSummaries(
    ws: string,
    projectSlug: string,
    opts: { silent?: boolean } = {},
  ): Promise<void> {
    const seq = (summariesLoadSeqByProject.get(projectSlug) ?? 0) + 1;
    summariesLoadSeqByProject.set(projectSlug, seq);
    const displaySeq = ++summariesDisplaySeq;
    const silent = opts.silent ?? false;

    error.value = null;
    if (!silent) {
      summariesDisplayProjectSlug = projectSlug;
      summariesLoadingDisplaySeq = displaySeq;
      loading.value = true;
      loadingByProject.value = { ...loadingByProject.value, [projectSlug]: true };
      summaries.value = [];
      summariesByProject.value = { ...summariesByProject.value, [projectSlug]: [] };
    }

    const { items, error: apiError } = await collectPaged<DocumentSummary>((cursor) =>
      wrappedClient.GET('/api/workspaces/{ws}/projects/{project_slug}/documents', {
        params: {
          path: { ws, project_slug: projectSlug },
          query: { limit: 200, ...(cursor !== undefined ? { cursor } : {}) },
        },
      }),
    );

    if (seq !== summariesLoadSeqByProject.get(projectSlug)) {
      if (!silent) {
        loadingByProject.value = { ...loadingByProject.value, [projectSlug]: false };
        if (displaySeq === summariesLoadingDisplaySeq) loading.value = false;
      }
      return;
    }

    if (!silent) {
      loadingByProject.value = { ...loadingByProject.value, [projectSlug]: false };
      if (displaySeq === summariesLoadingDisplaySeq) loading.value = false;
    }

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to load documents');
      return;
    }

    publishSummariesForProject(projectSlug, items);
  }

  async function loadBacklinks(ws: string, slug: string, cache?: BacklinksCacheOptions): Promise<void> {
    resetSecondaryTarget(ws, slug);
    deactivateBacklinksCache();
    const seq = ++backlinksLoadSeq;
    backlinksStatus.value = 'pending';
    backlinksError.value = null;

    const publish = (items: BacklinkSummary[]): void => {
      if (seq !== backlinksLoadSeq || !isSecondaryTarget(ws, slug)) return;

      backlinks.value = items;
      backlinksStatus.value = 'ready';
    };
    const load = async (): Promise<BacklinkSummary[]> => {
      const { items, error: apiError } = await collectPaged<BacklinkSummary>((cursor) =>
        wrappedClient.GET('/api/workspaces/{ws}/documents/{slug}/backlinks', {
          params: { path: { ws, slug }, query: { limit: 200, ...(cursor !== undefined ? { cursor } : {}) } },
        }),
      );

      if (apiError !== undefined) throw new Error(errorHint(apiError, 'Failed to load backlinks'));
      return items;
    };
    const key =
      cache === undefined
        ? null
        : buildCacheKey({
            principal: getResourceCachePrincipal(),
            workspaceId: cache.workspaceId,
            resourceKind: 'note-secondary',
            resourceId: slug,
            query: { type: 'backlinks' },
          });

    try {
      if (key === null || !resourceCache.isAvailable()) {
        publish(await load());
        return;
      }

      const request = {
        key,
        payloadSchema: backlinksSchema,
        tags: [`document:${slug}`, 'secondary:backlinks'],
        freshForMs: CACHE_CADENCE.secondary.freshForMs,
        activeForMs: CACHE_CADENCE.secondary.activeForMs,
        retentionForMs: 24 * 60 * 60 * 1000,
        load,
        publish,
        isCurrent: () => seq === backlinksLoadSeq && isSecondaryTarget(ws, slug),
      };
      activeBacklinksCacheKey = key;
      await resourceCache.hydrate(request);
      if (activeBacklinksCacheKey !== key || !request.isCurrent()) return;

      resourceCache.activate(request);
      await resourceCache.revalidate(request);
    } catch (error) {
      if (seq !== backlinksLoadSeq || !isSecondaryTarget(ws, slug)) return;

      backlinksStatus.value = 'error';
      backlinksError.value = error instanceof Error ? error.message : 'Failed to load backlinks';
    }
  }

  /**
   * Loads the first page of a document's comments (oldest-first). Mirrors the
   * task-comment thread in the task inspector, but keyed by document slug.
   */
  async function loadComments(ws: string, slug: string): Promise<void> {
    resetSecondaryTarget(ws, slug);
    const seq = ++commentsLoadSeq;
    commentsStatus.value = 'pending';
    commentsError.value = null;

    const { data, error: apiError } = await wrappedClient.GET(
      '/api/workspaces/{ws}/documents/{slug}/comments',
      { params: { path: { ws, slug } } },
    );

    if (seq !== commentsLoadSeq || !isSecondaryTarget(ws, slug)) return;

    if (apiError !== undefined || data === undefined) {
      comments.value = [];
      commentsCursor.value = null;
      commentsHasMore.value = false;
      commentsStatus.value = 'error';
      commentsError.value = errorHint(apiError, 'Failed to load comments');
      return;
    }

    const legacyItems = legacyCommentItems(data);
    if (legacyItems === null) {
      comments.value = [];
      commentsCursor.value = null;
      commentsHasMore.value = false;
      commentsStatus.value = 'error';
      commentsError.value = 'Received an unsupported full comment feed';
      return;
    }

    comments.value = legacyItems;
    commentsCursor.value = data.next_cursor ?? null;
    commentsHasMore.value = data.has_more;
    commentsStatus.value = 'ready';
  }

  /** Appends the next page of comments using the stored cursor. No-op at the end. */
  async function loadMoreComments(ws: string, slug: string): Promise<void> {
    if (!isSecondaryTarget(ws, slug) || !commentsHasMore.value || commentsCursor.value === null) {
      return;
    }

    const seq = ++commentsLoadSeq;
    const cursor = commentsCursor.value;
    commentsStatus.value = 'pending';
    commentsError.value = null;

    const { data, error: apiError } = await wrappedClient.GET(
      '/api/workspaces/{ws}/documents/{slug}/comments',
      { params: { path: { ws, slug }, query: { cursor } } },
    );

    if (seq !== commentsLoadSeq || !isSecondaryTarget(ws, slug)) return;

    if (apiError !== undefined || data === undefined) {
      commentsStatus.value = 'error';
      commentsError.value = errorHint(apiError, 'Failed to load comments');
      return;
    }

    const legacyItems = legacyCommentItems(data);
    if (legacyItems === null) {
      commentsStatus.value = 'error';
      commentsError.value = 'Received an unsupported full comment feed';
      return;
    }

    comments.value = [...comments.value, ...legacyItems];
    commentsCursor.value = data.next_cursor ?? null;
    commentsHasMore.value = data.has_more;
    commentsStatus.value = 'ready';
  }

  async function addComment(ws: string, slug: string, body: string): Promise<boolean> {
    const target = { workspaceSlug: ws, slug };
    const generation = commentsTargetEpoch;
    if (!isCurrentCommentsTarget(target, generation)) return false;

    error.value = null;

    const { data, error: apiError } = await wrappedClient.POST(
      '/api/workspaces/{ws}/documents/{slug}/comments',
      { params: { path: { ws, slug } }, body: { body } },
    );

    if (apiError !== undefined || data === undefined) {
      if (isCurrentCommentsTarget(target, generation)) {
        error.value = errorHint(apiError, 'Failed to add comment');
      }
      return false;
    }

    // Only reflect locally once the whole thread is paged in, so a newest-first
    // append can't land out of order or be re-fetched as a duplicate by a later
    // "Load more". It is persisted server-side regardless.
    if (isCurrentCommentsTarget(target, generation) && !commentsHasMore.value) {
      comments.value = [...comments.value, data];
    }
    return true;
  }

  async function removeComment(ws: string, slug: string, commentId: string): Promise<boolean> {
    const target = { workspaceSlug: ws, slug };
    const generation = commentsTargetEpoch;
    if (!isCurrentCommentsTarget(target, generation)) return false;

    error.value = null;

    const snapshot = [...comments.value];
    comments.value = comments.value.filter((c) => c.id !== commentId);

    const { error: apiError } = await wrappedClient.DELETE(
      '/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}',
      { params: { path: { ws, slug, comment_id: commentId } } },
    );

    if (apiError !== undefined) {
      if (isCurrentCommentsTarget(target, generation)) {
        comments.value = snapshot;
        error.value = errorHint(apiError, 'Failed to remove comment');
      }
      return false;
    }

    return true;
  }

  /** Edits a comment's body (author-only server-side); swaps the DTO in place. */
  async function editComment(ws: string, slug: string, commentId: string, body: string): Promise<boolean> {
    const target = { workspaceSlug: ws, slug };
    const generation = commentsTargetEpoch;
    if (!isCurrentCommentsTarget(target, generation)) return false;

    error.value = null;

    const { data, error: apiError } = await wrappedClient.PATCH(
      '/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}',
      { params: { path: { ws, slug, comment_id: commentId } }, body: { body } },
    );

    if (apiError !== undefined || data === undefined) {
      if (isCurrentCommentsTarget(target, generation)) {
        error.value = errorHint(apiError, 'Failed to edit comment');
      }
      return false;
    }

    const idx = comments.value.findIndex((c) => c.id === commentId);
    if (isCurrentCommentsTarget(target, generation) && idx !== -1) {
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
      '/api/workspaces/{ws}/projects/{project_slug}/documents',
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
    const { error: apiError } = await wrappedClient.PATCH('/api/workspaces/{ws}/documents/{slug}', {
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

  async function remove(
    ws: string,
    projectSlug: string,
    slug: string,
    cache?: { workspaceId: string },
  ): Promise<boolean> {
    const { error: apiError } = await wrappedClient.DELETE('/api/workspaces/{ws}/documents/{slug}', {
      params: { path: { ws, slug } },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to delete document');
      return false;
    }

    if (cache !== undefined) {
      await resourceCache.purgeTags(
        [`document:${slug}`, `project:${projectSlug}`],
        getResourceCachePrincipal(),
        cache.workspaceId,
      );
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
    const { error: apiError } = await wrappedClient.PATCH('/api/workspaces/{ws}/documents/{slug}/move', {
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
    const { error: apiError } = await wrappedClient.POST('/api/workspaces/{ws}/documents/{slug}/copy', {
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

  /**
   * Uploads a file as an attachment of the given document and returns the created
   * record (or null on failure, with `error` set). Backs the note editor's image
   * paste/drop flow, which then inserts the attachment's URL as Markdown.
   *
   * The endpoint takes the raw file bytes as the request body and the original
   * name in an `x-file-name` header — unlike the multipart task upload. utoipa
   * models neither the header nor the binary body (both are `never` in the
   * generated types), so the request init is assembled and cast locally.
   */
  async function uploadAttachment(ws: string, slug: string, file: File): Promise<AttachmentDto | null> {
    error.value = null;

    const { data, error: apiError } = await wrappedClient.POST(
      '/api/workspaces/{ws}/documents/{slug}/attachments',
      {
        params: { path: { ws, slug } },
        body: file,
        bodySerializer: (f: File) => f,
        headers: {
          'x-file-name': attachmentFileName(file),
          'Content-Type': file.type || 'application/octet-stream',
        },
      } as never,
    );

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to upload image');
      return null;
    }

    return data;
  }

  return {
    summaries,
    summariesByProject,
    backlinks,
    backlinksStatus,
    backlinksError,
    comments,
    commentsHasMore,
    commentsStatus,
    commentsError,
    loading,
    error,
    summariesFor,
    isProjectLoading,
    clearProjectBuckets,
    publishSummariesForProject,
    resetSecondaryTarget,
    clearSecondaryTarget,
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
    uploadAttachment,
  };
});
