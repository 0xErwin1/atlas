export type ImageUploadResult = string | { url: string; markdown?: string } | null;

export function imageUploadInsertion(
  result: ImageUploadResult,
  fileName: string,
  atLineStart: boolean,
): string | null {
  if (result === null) return null;

  if (typeof result !== 'string' && result.markdown !== undefined) return result.markdown;

  const url = typeof result === 'string' ? result : result.url;
  const alt = imageAlt(fileName);
  return `${atLineStart ? '' : '\n'}![${alt}](${url})\n`;
}

function imageAlt(fileName: string): string {
  return fileName
    .replace(/\.[^.]+$/, '')
    .replaceAll(']', '')
    .replaceAll('\n', ' ')
    .replaceAll('\r', ' ')
    .trim();
}
