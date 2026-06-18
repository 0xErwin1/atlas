import { slugify } from '@/lib/slugify';

export interface WikilinkTrigger {
  /** The partial query typed after the opening `[[`. */
  query: string;
  /** Absolute document position of the first `[` of the opening `[[`. */
  from: number;
}

const OPEN = '[[';

/**
 * Detects an active `[[` autocomplete trigger in the text immediately preceding
 * the cursor (REQ-W16).
 *
 * Returns the partial query and the position of the opening `[[` when the cursor
 * sits inside an unclosed `[[…` token on the current run of text, or `null` when
 * there is no active trigger (no `[[`, the link was already closed with `]]`, or
 * the query already contains a newline).
 *
 * `textBefore` is the plain text from the start of the current text run up to the
 * cursor; `cursorPos` is the absolute document position of the cursor, used to
 * compute the absolute `from` of the opening bracket.
 */
export function detectWikilinkTrigger(textBefore: string, cursorPos: number): WikilinkTrigger | null {
  const openIndex = textBefore.lastIndexOf(OPEN);
  if (openIndex === -1) return null;

  const query = textBefore.slice(openIndex + OPEN.length);

  if (query.includes(']') || query.includes('[') || query.includes('\n')) {
    return null;
  }

  return {
    query,
    from: cursorPos - (textBefore.length - openIndex),
  };
}

export interface WikilinkCandidate {
  title: string;
}

/**
 * Filters note candidates by the active query for the autocomplete dropdown.
 *
 * Matching is case-insensitive substring on the title. An empty query returns
 * all candidates (the dropdown opens immediately after `[[`). This is a
 * client-side convenience filter; the authoritative ranking comes from the
 * `search?type=note` backend call.
 */
export function filterWikilinkCandidates<T extends WikilinkCandidate>(candidates: T[], query: string): T[] {
  const needle = query.trim().toLowerCase();
  if (needle.length === 0) return candidates;

  return candidates.filter((c) => c.title.toLowerCase().includes(needle));
}

/**
 * A parsed wikilink reference. `id` is the stable target document UUID when the
 * link is id-bound (`[[uuid|Title]]`); `null` for a legacy/hand-typed
 * `[[Title]]` that is still resolved by title-slug.
 */
export interface WikilinkRef {
  id: string | null;
  title: string;
}

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/**
 * Parses the inner content of a `[[…]]` token. `[[uuid|Title]]` yields the stable
 * id plus the display title; anything else (including a non-uuid before a `|`) is
 * treated as a plain title. Mirrors the server's `parse_wikilink_target`.
 */
export function parseWikilinkInner(inner: string): WikilinkRef {
  const pipe = inner.indexOf('|');
  if (pipe !== -1) {
    const id = inner.slice(0, pipe).trim();
    const title = inner.slice(pipe + 1).trim();
    if (UUID_RE.test(id)) return { id, title };
  }
  return { id: null, title: inner.trim() };
}

/**
 * Serializes a reference back to wikilink markdown: `[[uuid|Title]]` when id-bound
 * (stable across renames) or `[[Title]]` for a title-only link.
 */
export function formatWikilink(ref: WikilinkRef): string {
  return ref.id !== null ? `[[${ref.id}|${ref.title}]]` : `[[${ref.title}]]`;
}

/**
 * Resolves a wikilink reference to the document route path. Id-bound links
 * navigate by the stable uuid (the document route resolves uuid or slug, then
 * canonicalizes to the pretty slug); title-only links fall back to the slug.
 */
export function wikilinkHref(ref: WikilinkRef): string {
  return ref.id !== null ? `/n/${ref.id}` : `/n/${slugify(ref.title)}`;
}
