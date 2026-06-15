const ALLOWED_TAG_RE = /^mark$/i;

/**
 * Strips all HTML tags except <mark> from a snippet string.
 * Safe against XSS: only the <mark> element passes through; all other tags
 * (including <script>, <img>, event attributes, etc.) are removed.
 * Used to render search result snippets from the API (REQ-W25).
 */
export function sanitizeSnippet(html: string): string {
  return html.replace(/<\/?([a-zA-Z][a-zA-Z0-9]*)\b[^>]*>/g, (match, tag: string) => {
    return ALLOWED_TAG_RE.test(tag) ? match : '';
  });
}
