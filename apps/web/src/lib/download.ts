import { getPlatformTransport } from '@/platform/transport';

/**
 * Saves downloaded bytes to disk on whichever platform is running.
 *
 * In the browser an object URL and a `download` anchor do the job. The desktop
 * webview cannot: it does not serve the API origin, and whether it honours an
 * object-URL download is not something Atlas controls, so the bytes are handed to
 * the host, which writes them into the user's downloads directory.
 *
 * Returns false when the file did not land, so callers can surface the failure
 * rather than leave the user believing a download happened.
 */
export async function saveDownload(blob: Blob, fileName: string): Promise<boolean> {
  const transport = getPlatformTransport();

  if (transport.isDesktop) {
    const bytes = new Uint8Array(await blob.arrayBuffer());
    const { error } = await transport.saveDownload(fileName, bytes);
    return error === undefined;
  }

  const objectUrl = URL.createObjectURL(blob);
  try {
    const anchor = document.createElement('a');
    anchor.href = objectUrl;
    anchor.download = fileName;
    anchor.click();
  } finally {
    URL.revokeObjectURL(objectUrl);
  }

  return true;
}
