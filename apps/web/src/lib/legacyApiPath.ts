/**
 * Render-time compatibility for attachment links embedded in stored content.
 *
 * Document and comment bodies written before the V2 cutover carry attachment
 * links in the retired `/api/workspaces/...` form, because the server's
 * `markdown`/`url` DTO fields used to emit that shape. The server keeps no
 * alias for it (user decision 2026-09-03), so the web maps such links to their
 * `/api/v2/acta/workspaces/...` form when they are rendered. Stored content is
 * never modified.
 */

const LEGACY_PREFIX = '/api/workspaces/';
const CURRENT_PREFIX = '/api/v2/acta/workspaces/';

// `scheme://host[:port]` or a scheme-relative `//host[:port]`, so an absolute
// URL is matched on its pathname rather than on its full text.
const AUTHORITY_RE = /^(?:[a-zA-Z][a-zA-Z0-9+.-]*:)?\/\/[^/?#]*/;

function isSameOrigin(url: string): boolean {
  const origin = globalThis.location?.origin;
  if (origin === undefined) return false;

  try {
    return new URL(url, origin).origin === origin;
  } catch {
    return false;
  }
}

/**
 * Maps a root-relative path starting with `/api/workspaces/`, or an absolute
 * URL on the app's own origin whose pathname starts with it, to the
 * `/api/v2/acta/workspaces/` form, preserving authority, query, and fragment.
 * Every other input (already-V2 paths, URLs on any other host, relative
 * paths, data URIs) is returned unchanged.
 */
export function toCurrentApiPath(path: string): string {
  const authority = AUTHORITY_RE.exec(path)?.[0] ?? '';
  const rest = path.slice(authority.length);

  if (!rest.startsWith(LEGACY_PREFIX)) return path;
  if (authority !== '' && !isSameOrigin(path)) return path;

  return `${authority}${CURRENT_PREFIX}${rest.slice(LEGACY_PREFIX.length)}`;
}
