import { type Ref, ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import type { ImageUploadResult } from '@/components/editor/imageUpload';
import type { CommentParentTarget } from '@/composables/useCommentFeed';
import { errorHint } from '@/lib/apiError';

type CommentAttachment = components['schemas']['CommentAttachmentDto'];

export type DraftAttachmentStatus = 'queued' | 'uploading' | 'uploaded' | 'deleting' | 'error';

export interface DraftAttachment {
  clientId: string;
  file: File;
  status: DraftAttachmentStatus;
  progress: number | null;
  error: string | null;
  attachment: CommentAttachment | null;
  uploadToken: string;
}

function formData(file: File): FormData {
  const form = new FormData();
  form.append('file', file);
  return form;
}

function makeClientId(): string {
  return crypto.randomUUID();
}

export function useCommentDraftAttachments(target: Ref<CommentParentTarget>) {
  const draftId = ref<string | null>(null);
  const attachments = ref<DraftAttachment[]>([]);
  const error = ref<string | null>(null);
  const createToken = makeClientId();
  const uploadPromises = new Map<string, Promise<ImageUploadResult>>();
  let draftCreation: Promise<string | null> | null = null;

  function isActive(entry: DraftAttachment): boolean {
    return attachments.value.some((candidate) => candidate.clientId === entry.clientId);
  }

  async function ensureDraft(): Promise<string | null> {
    if (draftId.value !== null) return draftId.value;
    if (draftCreation !== null) return draftCreation;

    const requestTarget = target.value;
    const creation = (async () => {
      try {
        const response =
          requestTarget.kind === 'task'
            ? await wrappedClient.POST('/api/workspaces/{ws}/tasks/{readable_id}/comment-drafts', {
                params: {
                  path: { ws: requestTarget.ws, readable_id: requestTarget.readableId },
                  header: { 'x-create-token': createToken },
                },
              })
            : await wrappedClient.POST('/api/workspaces/{ws}/documents/{slug}/comment-drafts', {
                params: {
                  path: { ws: requestTarget.ws, slug: requestTarget.slug },
                  header: { 'x-create-token': createToken },
                },
              });

        if (response.error !== undefined || response.data === undefined) {
          error.value = errorHint(response.error, 'Failed to create attachment draft');
          return null;
        }

        draftId.value = response.data.id;
        return response.data.id;
      } catch (cause) {
        error.value = errorHint(cause, 'Failed to create attachment draft');
        return null;
      }
    })();
    draftCreation = creation;

    try {
      return await creation;
    } finally {
      draftCreation = null;
    }
  }

  async function uploadEntry(entry: DraftAttachment): Promise<ImageUploadResult> {
    const id = await ensureDraft();
    if (id === null) {
      if (isActive(entry)) {
        entry.status = 'error';
        entry.error ??= error.value ?? 'Failed to create attachment draft';
      }
      return null;
    }

    entry.status = entry.status === 'deleting' ? 'deleting' : 'uploading';
    entry.progress = 0;
    entry.error = null;

    const requestTarget = target.value;

    try {
      const response =
        requestTarget.kind === 'task'
          ? await wrappedClient.POST(
              '/api/workspaces/{ws}/tasks/{readable_id}/comment-drafts/{draft_id}/attachments',
              {
                params: {
                  path: { ws: requestTarget.ws, readable_id: requestTarget.readableId, draft_id: id },
                  header: { 'x-upload-token': entry.uploadToken },
                },
                body: '',
                bodySerializer: () => formData(entry.file),
              },
            )
          : await uploadDocumentDraftAttachment(requestTarget, id, entry.file, entry.uploadToken);

      if (!isActive(entry)) return null;

      if (response.error !== undefined || response.data === undefined) {
        entry.status = 'error';
        entry.error = errorHint(response.error, 'Failed to upload attachment');
        return null;
      }

      entry.progress = 100;
      entry.attachment = response.data;
      if (response.data.url === null || response.data.url === undefined) {
        entry.status = 'error';
        entry.error = 'Attachment response omitted its canonical URL';
        return null;
      }
      if (entry.status === 'deleting') {
        await deleteAttachment(entry);
        return null;
      }
      entry.status = 'uploaded';
      return { url: response.data.url, markdown: response.data.markdown ?? undefined };
    } catch (cause) {
      if (isActive(entry)) {
        entry.status = 'error';
        entry.error = errorHint(cause, 'Failed to upload attachment');
      }
      return null;
    }
  }

  function upload(entry: DraftAttachment): Promise<ImageUploadResult> {
    const existing = uploadPromises.get(entry.clientId);
    if (existing !== undefined) return existing;

    const request = uploadEntry(entry).finally(() => {
      uploadPromises.delete(entry.clientId);
    });
    uploadPromises.set(entry.clientId, request);
    return request;
  }

  function enqueue(file: File): Promise<ImageUploadResult> {
    const entry: DraftAttachment = {
      clientId: makeClientId(),
      file,
      status: 'queued',
      progress: null,
      error: null,
      attachment: null,
      uploadToken: makeClientId(),
    };
    attachments.value = [...attachments.value, entry];
    const queued = attachments.value.at(-1);
    return queued === undefined ? Promise.resolve(null) : upload(queued);
  }

  async function retry(clientId: string): Promise<ImageUploadResult> {
    const entry = attachments.value.find((candidate) => candidate.clientId === clientId);
    if (entry === undefined || entry.status !== 'error') return null;
    return upload(entry);
  }

  async function remove(clientId: string): Promise<boolean> {
    const entry = attachments.value.find((candidate) => candidate.clientId === clientId);
    if (entry === undefined) return true;

    entry.status = 'deleting';
    entry.error = null;
    const inFlight = uploadPromises.get(entry.clientId);
    if (inFlight !== undefined) {
      await inFlight;
      if (!isActive(entry)) return true;
    }

    if (entry.attachment === null && draftId.value !== null) {
      await upload(entry);
      if (!isActive(entry)) return true;
    }

    if (entry.attachment === null || draftId.value === null) {
      attachments.value = attachments.value.filter((candidate) => candidate.clientId !== clientId);
      return true;
    }

    return deleteAttachment(entry);
  }

  async function deleteAttachment(entry: DraftAttachment): Promise<boolean> {
    const id = draftId.value;
    const attachment = entry.attachment;
    if (id === null || attachment === null) return false;
    const requestTarget = target.value;
    const response =
      requestTarget.kind === 'task'
        ? await wrappedClient.DELETE(
            '/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}',
            {
              params: {
                path: {
                  ws: requestTarget.ws,
                  readable_id: requestTarget.readableId,
                  comment_id: id,
                  attachment_id: attachment.id,
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
                  comment_id: id,
                  attachment_id: attachment.id,
                },
              },
            },
          );

    if (response.error !== undefined) {
      entry.error = errorHint(response.error, 'Failed to remove attachment');
      entry.status = 'error';
      return false;
    }

    attachments.value = attachments.value.filter((candidate) => candidate.clientId !== entry.clientId);
    return true;
  }

  async function discard(): Promise<boolean> {
    const id = draftId.value;
    if (id === null) {
      attachments.value = [];
      error.value = null;
      return true;
    }

    const requestTarget = target.value;
    const response =
      requestTarget.kind === 'task'
        ? await wrappedClient.DELETE('/api/workspaces/{ws}/tasks/{readable_id}/comment-drafts/{draft_id}', {
            params: { path: { ws: requestTarget.ws, readable_id: requestTarget.readableId, draft_id: id } },
          })
        : await wrappedClient.DELETE('/api/workspaces/{ws}/documents/{slug}/comment-drafts/{draft_id}', {
            params: { path: { ws: requestTarget.ws, slug: requestTarget.slug, draft_id: id } },
          });

    const status = (response.error as { status?: number } | undefined)?.status;
    if (response.error !== undefined && status !== 410) {
      error.value = errorHint(response.error, 'Failed to discard attachment draft');
      return false;
    }

    draftId.value = null;
    attachments.value = [];
    error.value = null;
    return true;
  }

  return { attachments, discard, draftId, enqueue, error, remove, retry, uploadImage: enqueue };
}

async function uploadDocumentDraftAttachment(
  target: Extract<CommentParentTarget, { kind: 'document' }>,
  draftId: string,
  file: File,
  uploadToken: string,
) {
  const bytes = Array.from(new Uint8Array(await file.arrayBuffer()));
  return wrappedClient.POST('/api/workspaces/{ws}/documents/{slug}/comment-drafts/{draft_id}/attachments', {
    params: {
      path: { ws: target.ws, slug: target.slug, draft_id: draftId },
      header: { 'x-file-name': file.name, 'x-upload-token': uploadToken },
    },
    body: bytes,
    bodySerializer: () => file,
    headers: { 'Content-Type': file.type || 'application/octet-stream' },
  });
}
