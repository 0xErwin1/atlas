import { computed, ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import { useLoadingMap } from '@/composables/useLoadingMap';
import { errorHint } from '@/lib/apiError';

type CommentListResponse = components['schemas']['CommentListResponseDto'];
type CommentFeedPage = components['schemas']['Page_CommentFeedEntryDto'];
type CommentFeedEntry = CommentFeedPage['items'][number];
type CommentAttachment = Omit<components['schemas']['CommentAttachmentDto'], 'sha256'>;
type CommentLinkTarget = components['schemas']['CommentLinkTargetDto'];

export type CommentParentTarget =
  | { kind: 'task'; ws: string; readableId: string }
  | { kind: 'document'; ws: string; slug: string };

export type AvailableCommentLinkTarget = Extract<CommentLinkTarget, { status: 'available' }>;
export type UnavailableCommentLinkTarget = { status: 'unavailable'; label: 'Recurso no disponible' };
export type NormalizedCommentLinkTarget = AvailableCommentLinkTarget | UnavailableCommentLinkTarget;

type CommentFeedEvent = Extract<CommentFeedEntry, { type: 'event' }>;
type NormalizedCommentFeedEvent = Omit<CommentFeedEvent, 'target'> & {
  target?: NormalizedCommentLinkTarget | null;
};

export type NormalizedCommentFeedEntry =
  | {
      type: 'comment';
      comment: components['schemas']['CommentDto'];
      links: Array<{ target: NormalizedCommentLinkTarget }>;
    }
  | NormalizedCommentFeedEvent;

type FeedStatus = 'idle' | 'pending' | 'ready' | 'error';

function sameTarget(left: CommentParentTarget | null, right: CommentParentTarget): boolean {
  if (left?.kind !== right.kind || left.ws !== right.ws) return false;

  return left.kind === 'task'
    ? left.readableId === (right.kind === 'task' ? right.readableId : '')
    : left.slug === (right.kind === 'document' ? right.slug : '');
}

function isFullCommentFeed(data: CommentListResponse): data is CommentFeedPage {
  return data.items.every((item) => 'type' in item);
}

function entryId(entry: NormalizedCommentFeedEntry): string {
  return entry.type === 'comment' ? `comment:${entry.comment.id}` : `event:${entry.id}`;
}

function omitAttachmentDigest(attachment: components['schemas']['CommentAttachmentDto']): CommentAttachment {
  const { sha256: _sha256, ...safeAttachment } = attachment;
  return safeAttachment;
}

function normalizeFeedEntries(entries: CommentFeedEntry[]): NormalizedCommentFeedEntry[] {
  return entries.map((entry) =>
    entry.type === 'comment'
      ? { ...entry, links: entry.links.map((link) => ({ target: normalizeCommentLinkTarget(link.target) })) }
      : normalizeCommentFeedEvent(entry),
  );
}

function normalizeCommentFeedEvent(entry: CommentFeedEvent): NormalizedCommentFeedEvent {
  const { target, ...event } = entry;

  if (target === undefined) return event;
  if (target === null) return { ...event, target: null };

  return { ...event, target: normalizeCommentLinkTarget(target) };
}

export function normalizeCommentLinkTarget(target: CommentLinkTarget): NormalizedCommentLinkTarget {
  if (target.status === 'unavailable') {
    return { status: 'unavailable', label: 'Recurso no disponible' };
  }

  return target;
}

export function useCommentFeed() {
  const entries = ref<NormalizedCommentFeedEntry[]>([]);
  const cursor = ref<string | null>(null);
  const hasMore = ref(false);
  const status = ref<FeedStatus>('idle');
  const error = ref<string | null>(null);
  const attachments = ref<Record<string, CommentAttachment[]>>({});
  const attachmentError = ref<Record<string, string>>({});
  const attachmentListLoading = useLoadingMap();
  const attachmentUploadLoading = useLoadingMap();
  const attachmentDownloadLoading = useLoadingMap();
  const attachmentDeleteLoading = useLoadingMap();
  const target = ref<CommentParentTarget | null>(null);
  let generation = 0;
  let requestSequence = 0;

  const isLoading = computed(() => status.value === 'pending');

  function isCurrent(
    requestTarget: CommentParentTarget,
    requestGeneration: number,
    sequence: number,
  ): boolean {
    return (
      generation === requestGeneration &&
      requestSequence === sequence &&
      sameTarget(target.value, requestTarget)
    );
  }

  function reset(nextTarget: CommentParentTarget): number {
    if (!sameTarget(target.value, nextTarget)) {
      generation += 1;
      target.value = nextTarget;
      entries.value = [];
      cursor.value = null;
      hasMore.value = false;
      attachments.value = {};
      attachmentError.value = {};
      attachmentListLoading.clear();
      attachmentUploadLoading.clear();
      attachmentDownloadLoading.clear();
      attachmentDeleteLoading.clear();
    }

    return generation;
  }

  async function requestFeed(
    requestTarget: CommentParentTarget,
    requestCursor?: string,
  ): Promise<{ data?: CommentListResponse; error?: unknown }> {
    const query = {
      feed: 'full' as const,
      ...(requestCursor === undefined ? {} : { cursor: requestCursor }),
    };

    if (requestTarget.kind === 'task') {
      return wrappedClient.GET('/api/workspaces/{ws}/tasks/{readable_id}/comments', {
        params: { path: { ws: requestTarget.ws, readable_id: requestTarget.readableId }, query },
      });
    }

    return wrappedClient.GET('/api/workspaces/{ws}/documents/{slug}/comments', {
      params: { path: { ws: requestTarget.ws, slug: requestTarget.slug }, query },
    });
  }

  function applyPage(page: CommentFeedPage, append: boolean): void {
    const incoming = normalizeFeedEntries(page.items);
    const merged = append ? [...entries.value, ...incoming] : incoming;
    const ids = new Set<string>();
    entries.value = merged.filter((entry) => {
      const id = entryId(entry);
      if (ids.has(id)) return false;
      ids.add(id);
      return true;
    });
    cursor.value = page.next_cursor ?? null;
    hasMore.value = page.has_more;
  }

  async function load(nextTarget: CommentParentTarget): Promise<void> {
    const requestGeneration = reset(nextTarget);
    const sequence = ++requestSequence;
    status.value = 'pending';
    error.value = null;

    try {
      const { data, error: apiError } = await requestFeed(nextTarget);
      if (!isCurrent(nextTarget, requestGeneration, sequence)) return;

      if (apiError !== undefined || data === undefined || !isFullCommentFeed(data)) {
        entries.value = [];
        cursor.value = null;
        hasMore.value = false;
        status.value = 'error';
        error.value = errorHint(apiError, 'Failed to load comments');
        return;
      }

      applyPage(data, false);
      status.value = 'ready';
    } catch (cause) {
      if (!isCurrent(nextTarget, requestGeneration, sequence)) return;

      entries.value = [];
      cursor.value = null;
      hasMore.value = false;
      status.value = 'error';
      error.value = errorHint(cause, 'Failed to load comments');
    }
  }

  async function loadMore(requestTarget: CommentParentTarget): Promise<void> {
    if (!sameTarget(target.value, requestTarget) || !hasMore.value || cursor.value === null) return;

    const requestGeneration = generation;
    const sequence = ++requestSequence;
    const requestCursor = cursor.value;
    status.value = 'pending';
    error.value = null;

    try {
      const { data, error: apiError } = await requestFeed(requestTarget, requestCursor);
      if (!isCurrent(requestTarget, requestGeneration, sequence)) return;

      if (apiError !== undefined || data === undefined || !isFullCommentFeed(data)) {
        status.value = 'error';
        error.value = errorHint(apiError, 'Failed to load comments');
        return;
      }

      applyPage(data, true);
      status.value = 'ready';
    } catch (cause) {
      if (!isCurrent(requestTarget, requestGeneration, sequence)) return;

      status.value = 'error';
      error.value = errorHint(cause, 'Failed to load comments');
    }
  }

  function currentAttachmentTarget(requestTarget: CommentParentTarget, requestGeneration: number): boolean {
    return generation === requestGeneration && sameTarget(target.value, requestTarget);
  }

  function setAttachmentError(commentId: string, message: string | null): void {
    const next = { ...attachmentError.value };
    if (message === null) delete next[commentId];
    else next[commentId] = message;
    attachmentError.value = next;
  }

  async function listAttachmentRequest(
    requestTarget: CommentParentTarget,
    commentId: string,
  ): Promise<{ data?: components['schemas']['CommentAttachmentDto'][]; error?: unknown }> {
    if (requestTarget.kind === 'task') {
      return wrappedClient.GET('/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments', {
        params: {
          path: { ws: requestTarget.ws, readable_id: requestTarget.readableId, comment_id: commentId },
        },
      });
    }

    return wrappedClient.GET('/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments', {
      params: { path: { ws: requestTarget.ws, slug: requestTarget.slug, comment_id: commentId } },
    });
  }

  async function loadAttachments(requestTarget: CommentParentTarget, commentId: string): Promise<void> {
    const requestGeneration = reset(requestTarget);
    attachmentListLoading.set(commentId, true);
    setAttachmentError(commentId, null);

    try {
      const { data, error: apiError } = await listAttachmentRequest(requestTarget, commentId);
      if (!currentAttachmentTarget(requestTarget, requestGeneration)) return;

      if (apiError !== undefined || data === undefined) {
        setAttachmentError(commentId, errorHint(apiError, 'Failed to load comment attachments'));
        return;
      }

      attachments.value = { ...attachments.value, [commentId]: data.map(omitAttachmentDigest) };
    } catch (cause) {
      if (currentAttachmentTarget(requestTarget, requestGeneration)) {
        setAttachmentError(commentId, errorHint(cause, 'Failed to load comment attachments'));
      }
    } finally {
      if (currentAttachmentTarget(requestTarget, requestGeneration))
        attachmentListLoading.set(commentId, false);
    }
  }

  function formData(file: File): FormData {
    const form = new FormData();
    form.append('file', file);
    return form;
  }

  async function uploadAttachment(
    requestTarget: CommentParentTarget,
    commentId: string,
    file: File,
  ): Promise<CommentAttachment | null> {
    const requestGeneration = reset(requestTarget);
    attachmentUploadLoading.set(commentId, true);
    setAttachmentError(commentId, null);

    try {
      const response =
        requestTarget.kind === 'task'
          ? await wrappedClient.POST(
              '/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments',
              {
                params: {
                  path: {
                    ws: requestTarget.ws,
                    readable_id: requestTarget.readableId,
                    comment_id: commentId,
                  },
                },
                body: '',
                bodySerializer: () => formData(file),
              },
            )
          : await uploadDocumentCommentAttachment(requestTarget, commentId, file);

      if (!currentAttachmentTarget(requestTarget, requestGeneration)) return null;

      if (response.error !== undefined || response.data === undefined) {
        setAttachmentError(commentId, errorHint(response.error, 'Failed to upload comment attachment'));
        return null;
      }

      const uploaded = omitAttachmentDigest(response.data);
      const current = attachments.value[commentId] ?? [];
      attachments.value = { ...attachments.value, [commentId]: [...current, uploaded] };
      return uploaded;
    } catch (cause) {
      if (currentAttachmentTarget(requestTarget, requestGeneration)) {
        setAttachmentError(commentId, errorHint(cause, 'Failed to upload comment attachment'));
      }
      return null;
    } finally {
      if (currentAttachmentTarget(requestTarget, requestGeneration))
        attachmentUploadLoading.set(commentId, false);
    }
  }

  async function downloadAttachment(
    requestTarget: CommentParentTarget,
    commentId: string,
    attachmentId: string,
  ): Promise<Blob | null> {
    const requestGeneration = reset(requestTarget);
    const loadingId = `${commentId}:${attachmentId}`;
    attachmentDownloadLoading.set(loadingId, true);
    setAttachmentError(commentId, null);

    try {
      const response =
        requestTarget.kind === 'task'
          ? await wrappedClient.GET(
              '/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}/content',
              {
                params: {
                  path: {
                    ws: requestTarget.ws,
                    readable_id: requestTarget.readableId,
                    comment_id: commentId,
                    attachment_id: attachmentId,
                  },
                },
                parseAs: 'blob',
              },
            )
          : await wrappedClient.GET(
              '/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments/{attachment_id}',
              {
                params: {
                  path: {
                    ws: requestTarget.ws,
                    slug: requestTarget.slug,
                    comment_id: commentId,
                    attachment_id: attachmentId,
                  },
                },
                parseAs: 'blob',
              },
            );

      if (!currentAttachmentTarget(requestTarget, requestGeneration)) return null;

      if (response.error !== undefined || response.data === undefined) {
        setAttachmentError(commentId, errorHint(response.error, 'Failed to download comment attachment'));
        return null;
      }

      return response.data;
    } catch (cause) {
      if (currentAttachmentTarget(requestTarget, requestGeneration)) {
        setAttachmentError(commentId, errorHint(cause, 'Failed to download comment attachment'));
      }
      return null;
    } finally {
      if (currentAttachmentTarget(requestTarget, requestGeneration))
        attachmentDownloadLoading.set(loadingId, false);
    }
  }

  async function deleteAttachment(
    requestTarget: CommentParentTarget,
    commentId: string,
    attachmentId: string,
  ): Promise<boolean> {
    const requestGeneration = reset(requestTarget);
    const loadingId = `${commentId}:${attachmentId}`;
    attachmentDeleteLoading.set(loadingId, true);
    setAttachmentError(commentId, null);

    try {
      const response =
        requestTarget.kind === 'task'
          ? await wrappedClient.DELETE(
              '/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}',
              {
                params: {
                  path: {
                    ws: requestTarget.ws,
                    readable_id: requestTarget.readableId,
                    comment_id: commentId,
                    attachment_id: attachmentId,
                  },
                },
              },
            )
          : await wrappedClient.DELETE(
              '/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments/{attachment_id}',
              {
                params: {
                  path: {
                    ws: requestTarget.ws,
                    slug: requestTarget.slug,
                    comment_id: commentId,
                    attachment_id: attachmentId,
                  },
                },
              },
            );

      if (!currentAttachmentTarget(requestTarget, requestGeneration)) return false;

      if (response.error !== undefined) {
        setAttachmentError(commentId, errorHint(response.error, 'Failed to delete comment attachment'));
        return false;
      }

      const current = attachments.value[commentId] ?? [];
      attachments.value = {
        ...attachments.value,
        [commentId]: current.filter((item) => item.id !== attachmentId),
      };
      return true;
    } catch (cause) {
      if (currentAttachmentTarget(requestTarget, requestGeneration)) {
        setAttachmentError(commentId, errorHint(cause, 'Failed to delete comment attachment'));
      }
      return false;
    } finally {
      if (currentAttachmentTarget(requestTarget, requestGeneration))
        attachmentDeleteLoading.set(loadingId, false);
    }
  }

  return {
    entries,
    cursor,
    hasMore,
    status,
    error,
    isLoading,
    attachments,
    attachmentError,
    isAttachmentListLoading: attachmentListLoading.isLoading,
    isAttachmentUploadLoading: attachmentUploadLoading.isLoading,
    isAttachmentDownloadLoading: attachmentDownloadLoading.isLoading,
    isAttachmentDeleteLoading: attachmentDeleteLoading.isLoading,
    load,
    loadMore,
    loadAttachments,
    uploadAttachment,
    downloadAttachment,
    deleteAttachment,
  };
}

async function uploadDocumentCommentAttachment(
  target: Extract<CommentParentTarget, { kind: 'document' }>,
  commentId: string,
  file: File,
) {
  const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));

  return wrappedClient.POST('/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments', {
    params: {
      path: { ws: target.ws, slug: target.slug, comment_id: commentId },
      header: { 'x-file-name': file.name },
    },
    body: bytes,
    bodySerializer: () => file,
    headers: {
      'Content-Type': file.type || 'application/octet-stream',
    },
  });
}
