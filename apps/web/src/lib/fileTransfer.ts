/**
 * Helpers for pulling files out of native drag-and-drop and clipboard events,
 * shared by the task attachment dropzone and the note image paste/drop flow.
 * Pure functions: no DOM mutation, no upload logic.
 */

/** Files carried by a drag-and-drop `DataTransfer`, in drop order. */
export function filesFromDataTransfer(data: DataTransfer | null): File[] {
  if (data === null) return [];
  return Array.from(data.files);
}

/** Files carried by a clipboard paste (e.g. a pasted screenshot), in order. */
export function filesFromClipboard(data: DataTransfer | null): File[] {
  if (data === null) return [];

  const files: File[] = [];
  for (const item of Array.from(data.items)) {
    if (item.kind !== 'file') continue;

    const file = item.getAsFile();
    if (file !== null) files.push(file);
  }
  return files;
}

export function isImageFile(file: File): boolean {
  return file.type.startsWith('image/');
}

/**
 * A safe, human-readable name for an uploaded file. Clipboard images often arrive
 * nameless, so one is synthesized from the MIME subtype; non-ASCII characters are
 * folded and quotes dropped so the value stays usable as an HTTP header.
 */
export function attachmentFileName(file: File): string {
  const cleaned = file.name
    .trim()
    .replace(/[^\x20-\x7E]/g, '_')
    .replace(/"/g, '');

  if (cleaned !== '') return cleaned;

  const subtype = file.type.split('/')[1] ?? 'bin';
  return `pasted-image.${subtype}`;
}
