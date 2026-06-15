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
 * Resolves a wikilink title to the document route path, using the server-parity
 * slugify so the navigation target matches the slug the backend assigned
 * (REQ-W16).
 */
export function wikilinkTarget(title: string): string {
  return `/n/${slugify(title)}`;
}
