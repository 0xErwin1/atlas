import { type Ref, ref, watch } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import {
  deleteCommentAttachment,
  downloadCommentAttachment,
  uploadDocumentCommentAttachment,
} from '@/composables/commentAttachmentTransfers';
import type { CommentParentTarget } from '@/composables/useCommentFeed';
import { useLoadingMap } from '@/composables/useLoadingMap';
import { errorHint } from '@/lib/apiError';

type CommentAttachment = Omit<components['schemas']['CommentAttachmentDto'], 'sha256'>;
type CommentEntry = { type: string; comment?: { id: string } };

function sameTarget(left: CommentParentTarget | null, right: CommentParentTarget): boolean {
  if (left?.kind !== right.kind || left.ws !== right.ws) return false;

  return left.kind === 'task'
    ? left.readableId === (right.kind === 'task' ? right.readableId : '')
    : left.slug === (right.kind === 'document' ? right.slug : '');
}

function omitAttachmentDigest(attachment: components['schemas']['CommentAttachmentDto']): CommentAttachment {
  const { sha256: _sha256, ...safeAttachment } = attachment;
  return safeAttachment;
}

function formData(file: File): FormData {
  const form = new FormData();
  form.append('file', file);
  return form;
}

export function useCommentAttachments(target: Ref<CommentParentTarget>, entries: Ref<CommentEntry[]>) {
  const items = ref<Record<string, CommentAttachment[]>>({});
  const error = ref<Record<string, string>>({});
  const listing = useLoadingMap();
  const uploading = useLoadingMap();
  const downloading = useLoadingMap();
  const deleting = useLoadingMap();
  const loadedCommentIds = new Set<string>();
  const mutationRevision = new Map<string, number>();
  let activeTarget: CommentParentTarget | null = null;
  let generation = 0;

  function reset(nextTarget: CommentParentTarget): number {
    if (!sameTarget(activeTarget, nextTarget)) {
      activeTarget = nextTarget;
      generation += 1;
      items.value = {};
      error.value = {};
      loadedCommentIds.clear();
      mutationRevision.clear();
      listing.clear();
      uploading.clear();
      downloading.clear();
      deleting.clear();
    }

    return generation;
  }

  function isCurrent(requestTarget: CommentParentTarget, requestGeneration: number): boolean {
    return (
      generation === requestGeneration &&
      sameTarget(activeTarget, requestTarget) &&
      sameTarget(target.value, requestTarget)
    );
  }

  function advanceMutationRevision(commentId: string): number {
    const next = (mutationRevision.get(commentId) ?? 0) + 1;
    mutationRevision.set(commentId, next);
    return next;
  }

  function isCurrentListRequest(
    requestTarget: CommentParentTarget,
    requestGeneration: number,
    commentId: string,
    requestMutationRevision: number,
  ): boolean {
    return (
      isCurrent(requestTarget, requestGeneration) &&
      (mutationRevision.get(commentId) ?? 0) === requestMutationRevision
    );
  }

  function setError(commentId: string, message: string | null): void {
    const next = { ...error.value };
    if (message === null) delete next[commentId];
    else next[commentId] = message;
    error.value = next;
  }

  async function listRequest(requestTarget: CommentParentTarget, commentId: string) {
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

  async function reload(commentId: string): Promise<void> {
    if (listing.isLoading(commentId)) return;

    const requestTarget = target.value;
    const requestGeneration = reset(requestTarget);
    const requestMutationRevision = mutationRevision.get(commentId) ?? 0;
    listing.set(commentId, true);
    setError(commentId, null);

    try {
      const { data, error: apiError } = await listRequest(requestTarget, commentId);
      if (!isCurrentListRequest(requestTarget, requestGeneration, commentId, requestMutationRevision)) return;

      if (apiError !== undefined || data === undefined) {
        setError(commentId, errorHint(apiError, 'Failed to load comment attachments'));
        return;
      }

      items.value = { ...items.value, [commentId]: data.map(omitAttachmentDigest) };
    } catch (cause) {
      if (isCurrentListRequest(requestTarget, requestGeneration, commentId, requestMutationRevision)) {
        setError(commentId, errorHint(cause, 'Failed to load comment attachments'));
      }
    } finally {
      if (isCurrent(requestTarget, requestGeneration)) listing.set(commentId, false);
    }
  }

  async function upload(commentId: string, file: File): Promise<CommentAttachment | null> {
    if (uploading.isLoading(commentId)) return null;

    const requestTarget = target.value;
    const requestGeneration = reset(requestTarget);
    uploading.set(commentId, true);
    setError(commentId, null);

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

      if (!isCurrent(requestTarget, requestGeneration)) return null;

      if (response.error !== undefined || response.data === undefined) {
        setError(commentId, errorHint(response.error, 'Failed to upload comment attachment'));
        return null;
      }

      const uploaded = omitAttachmentDigest(response.data);
      advanceMutationRevision(commentId);
      items.value = { ...items.value, [commentId]: [...(items.value[commentId] ?? []), uploaded] };
      return uploaded;
    } catch (cause) {
      if (isCurrent(requestTarget, requestGeneration)) {
        setError(commentId, errorHint(cause, 'Failed to upload comment attachment'));
      }
      return null;
    } finally {
      if (isCurrent(requestTarget, requestGeneration)) uploading.set(commentId, false);
    }
  }

  function contentUrl(commentId: string, attachmentId: string): string {
    const requestTarget = target.value;
    return requestTarget.kind === 'task'
      ? `/api/workspaces/${requestTarget.ws}/tasks/${requestTarget.readableId}/comments/${commentId}/attachments/${attachmentId}/content`
      : `/api/workspaces/${requestTarget.ws}/documents/${requestTarget.slug}/comments/${commentId}/attachments/${attachmentId}`;
  }

  async function download(commentId: string, attachmentId: string): Promise<Blob | null> {
    const loadingId = `${commentId}:${attachmentId}`;
    if (downloading.isLoading(loadingId)) return null;

    const requestTarget = target.value;
    const requestGeneration = reset(requestTarget);
    downloading.set(loadingId, true);
    setError(commentId, null);

    try {
      const response = await downloadCommentAttachment(requestTarget, commentId, attachmentId);
      if (!isCurrent(requestTarget, requestGeneration)) return null;

      if (response.error !== undefined || response.data === undefined) {
        setError(commentId, errorHint(response.error, 'Failed to download comment attachment'));
        return null;
      }

      const objectUrl = URL.createObjectURL(response.data);
      const anchor = document.createElement('a');
      anchor.href = objectUrl;
      anchor.download =
        items.value[commentId]?.find((item) => item.id === attachmentId)?.file_name ?? 'attachment';
      anchor.click();
      URL.revokeObjectURL(objectUrl);
      return response.data;
    } catch (cause) {
      if (isCurrent(requestTarget, requestGeneration)) {
        setError(commentId, errorHint(cause, 'Failed to download comment attachment'));
      }
      return null;
    } finally {
      if (isCurrent(requestTarget, requestGeneration)) downloading.set(loadingId, false);
    }
  }

  async function remove(commentId: string, attachmentId: string): Promise<boolean> {
    const loadingId = `${commentId}:${attachmentId}`;
    if (deleting.isLoading(loadingId)) return false;

    const requestTarget = target.value;
    const requestGeneration = reset(requestTarget);
    deleting.set(loadingId, true);
    setError(commentId, null);

    try {
      const response = await deleteCommentAttachment(requestTarget, commentId, attachmentId);
      if (!isCurrent(requestTarget, requestGeneration)) return false;

      if (response.error !== undefined) {
        setError(commentId, errorHint(response.error, 'Failed to delete comment attachment'));
        return false;
      }

      advanceMutationRevision(commentId);
      items.value = {
        ...items.value,
        [commentId]: (items.value[commentId] ?? []).filter((item) => item.id !== attachmentId),
      };
      return true;
    } catch (cause) {
      if (isCurrent(requestTarget, requestGeneration)) {
        setError(commentId, errorHint(cause, 'Failed to delete comment attachment'));
      }
      return false;
    } finally {
      if (isCurrent(requestTarget, requestGeneration)) deleting.set(loadingId, false);
    }
  }

  function loadEntries(nextEntries: CommentEntry[]): void {
    for (const entry of nextEntries) {
      if (entry.type !== 'comment' || entry.comment === undefined || loadedCommentIds.has(entry.comment.id))
        continue;
      loadedCommentIds.add(entry.comment.id);
      void reload(entry.comment.id);
    }
  }

  watch(
    target,
    (nextTarget) => {
      reset(nextTarget);
      loadEntries(entries.value);
    },
    { immediate: true },
  );

  watch(entries, loadEntries, { deep: true, immediate: true });

  return {
    items,
    error,
    isListing: listing.isLoading,
    isUploading: uploading.isLoading,
    isDownloading: downloading.isLoading,
    isDeleting: deleting.isLoading,
    reload,
    upload,
    download,
    delete: remove,
    contentUrl,
  };
}
