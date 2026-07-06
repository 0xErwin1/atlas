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

/**
 * URL schemes permitted for user-authored links and images. Everything else —
 * notably `javascript:`, `data:`, and `vbscript:` — is rejected so a crafted
 * `[text](javascript:...)` link or `![](...)` image cannot execute script in the
 * victim's authenticated, same-origin session (stored DOM XSS).
 */
const SAFE_URL_SCHEMES = new Set(['http:', 'https:', 'mailto:']);

const URL_SCHEME_RE = /^([a-zA-Z][a-zA-Z0-9+.-]*):/;

/**
 * Validates a user-authored URL for safe use as an anchor `href` or image `src`,
 * returning a normalized URL when safe and `null` when it must be neutralized.
 *
 * A URL is safe when it is relative/anchor/scheme-relative (no scheme, or begins
 * with `/`, `./`, `../`, `#`) or carries an allowlisted scheme (`http:`,
 * `https:`, `mailto:`).
 *
 * Normalization strips leading/trailing whitespace and every ASCII control
 * character (tabs, newlines, and other C0/DEL bytes) BEFORE inspecting the
 * scheme, because browsers silently drop those characters when parsing a URL —
 * so `java\tscript:alert(1)` would run as `javascript:` unless neutralized here.
 * The returned value is the SAME normalized string that was validated, so what
 * the caller emits is exactly what passed the check.
 */
export function safeUrl(raw: string): string | null {
  // biome-ignore lint/suspicious/noControlCharactersInRegex: browsers strip these from URLs, so they must be removed before the scheme check to avoid a `java\tscript:` bypass.
  const normalized = raw.replace(/[\u0000-\u001f\u007f]/g, '').trim();

  if (normalized.length === 0) return null;

  const match = URL_SCHEME_RE.exec(normalized);
  if (match === null) return normalized;

  const scheme = `${match[1]?.toLowerCase()}:`;
  return SAFE_URL_SCHEMES.has(scheme) ? normalized : null;
}
