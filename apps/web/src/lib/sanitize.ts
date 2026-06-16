const TAG_RE = /<\/?[a-zA-Z][a-zA-Z0-9]*\b[^>]*>/g;
const MARK_OPEN_RE = /^<mark\b/i;
const MARK_CLOSE_RE = /^<\/mark\b/i;

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

/**
 * Sanitizes a search-result snippet for safe rendering via v-html (REQ-W25).
 *
 * Postgres `ts_headline` does NOT HTML-escape the document body — it only wraps
 * matches with the literal `<mark>`/`</mark>` markers we asked for. So the source
 * may contain attacker-controlled markup, including attribute-bearing
 * `<mark onmouseover=...>`. This sanitizer is the only line of defence.
 *
 * The contract: the ONLY live HTML that survives is a bare `<mark>`/`</mark>`
 * highlight (all attributes discarded). Every other recognised tag is dropped,
 * and every text segment between tags is HTML-escaped, so no other markup —
 * stray angle brackets, ampersands, quotes, or a nested `<script>` — can ever
 * become live HTML in the browser.
 */
export function sanitizeSnippet(html: string): string {
  let result = '';
  let lastIndex = 0;

  for (const match of html.matchAll(TAG_RE)) {
    const tag = match[0];
    const start = match.index ?? 0;

    result += escapeHtml(html.slice(lastIndex, start));

    if (MARK_OPEN_RE.test(tag)) {
      result += '<mark>';
    } else if (MARK_CLOSE_RE.test(tag)) {
      result += '</mark>';
    }

    lastIndex = start + tag.length;
  }

  result += escapeHtml(html.slice(lastIndex));

  return result;
}
