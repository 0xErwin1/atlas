/**
 * Addressing and Markdown rendering for task attachments.
 *
 * The content endpoint is cookie-authenticated and same-origin; it sets
 * Content-Disposition so a plain link downloads rather than navigates.
 */
export function taskAttachmentContentUrl(ws: string, readableId: string, attachmentId: string): string {
  const segments = [ws, readableId, attachmentId].map(encodeURIComponent);
  return `/api/v2/acta/workspaces/${segments[0]}/tasks/${segments[1]}/attachments/${segments[2]}/content`;
}

/**
 * Renders an attachment as the Markdown that references it from a body: an image
 * embed for image content types, a plain link otherwise. Mirrors the server's
 * `comment_attachment_markdown` (crates/atlas_server/src/routes/mod.rs) so a task
 * attachment reads the same as a comment attachment wherever it is pasted.
 */
export function attachmentMarkdown(fileName: string, contentType: string, url: string): string {
  const isImage = contentType.startsWith('image/');
  const named = isImage ? stripExtension(fileName) : fileName;

  // A `]` or newline inside the label would terminate it early and leave the rest
  // of the file name as loose text next to a broken link.
  const label = named
    .replaceAll(']', '')
    .replace(/[\r\n]/g, ' ')
    .trim();

  return isImage ? `![${label}](${url})` : `[${label}](${url})`;
}

function stripExtension(fileName: string): string {
  const dot = fileName.lastIndexOf('.');
  return dot === -1 ? fileName : fileName.slice(0, dot);
}
