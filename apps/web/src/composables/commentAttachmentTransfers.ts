import { wrappedClient } from '@/api/wrapper';
import type { CommentParentTarget } from '@/composables/useCommentFeed';

export async function uploadDocumentCommentAttachment(
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
    headers: { 'Content-Type': file.type || 'application/octet-stream' },
  });
}

export async function downloadCommentAttachment(
  target: CommentParentTarget,
  commentId: string,
  attachmentId: string,
) {
  if (target.kind === 'task') {
    return wrappedClient.GET(
      '/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}/content',
      {
        params: {
          path: {
            ws: target.ws,
            readable_id: target.readableId,
            comment_id: commentId,
            attachment_id: attachmentId,
          },
        },
        parseAs: 'blob',
      },
    );
  }

  return wrappedClient.GET(
    '/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments/{attachment_id}',
    {
      params: {
        path: { ws: target.ws, slug: target.slug, comment_id: commentId, attachment_id: attachmentId },
      },
      parseAs: 'blob',
    },
  );
}

export async function deleteCommentAttachment(
  target: CommentParentTarget,
  commentId: string,
  attachmentId: string,
) {
  if (target.kind === 'task') {
    return wrappedClient.DELETE(
      '/api/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}',
      {
        params: {
          path: {
            ws: target.ws,
            readable_id: target.readableId,
            comment_id: commentId,
            attachment_id: attachmentId,
          },
        },
      },
    );
  }

  return wrappedClient.DELETE(
    '/api/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments/{attachment_id}',
    {
      params: {
        path: { ws: target.ws, slug: target.slug, comment_id: commentId, attachment_id: attachmentId },
      },
    },
  );
}
