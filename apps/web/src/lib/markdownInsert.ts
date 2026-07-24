/**
 * Wraps host-inserted markdown so it lands as its own block: a leading break when
 * the caret sits mid-line, and a trailing break so following text starts fresh.
 */
export function blockInsertion(markdown: string, atLineStart: boolean): string {
  return `${atLineStart ? '' : '\n'}${markdown}\n`;
}
