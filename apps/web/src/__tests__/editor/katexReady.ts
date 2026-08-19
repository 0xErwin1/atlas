/**
 * Waits for the live-preview extension's lazy KaTeX import to resolve and for the
 * math widgets it paints asynchronously to be filled in.
 */
export async function flushKatex(): Promise<void> {
  await import('katex');
  await new Promise((resolve) => setTimeout(resolve, 0));
}
