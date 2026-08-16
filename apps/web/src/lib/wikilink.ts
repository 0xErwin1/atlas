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
 * `search` backend call.
 */
export function filterWikilinkCandidates<T extends WikilinkCandidate>(candidates: T[], query: string): T[] {
  const needle = query.trim().toLowerCase();
  if (needle.length === 0) return candidates;

  return candidates.filter((c) => c.title.toLowerCase().includes(needle));
}

/** The kinds a typed wikilink can address, in the order they appear in text. */
export const WIKILINK_KINDS = ['task', 'note', 'file'] as const;

export type WikilinkKind = (typeof WIKILINK_KINDS)[number];

/**
 * What a `[[…]]` link points at. The typed forms address their target by the
 * identifier a reader already knows, so no UUID appears in the markdown; the
 * two untyped forms are what every link written before typed links meant.
 *
 * Mirrors the server's `WikilinkTarget`.
 */
export type WikilinkTarget =
  | { kind: 'task'; readableId: string }
  | { kind: 'note'; slug: string }
  | { kind: 'file'; fileName: string }
  | { kind: 'document'; id: string }
  | { kind: 'title' };

/** A parsed wikilink: where it points and the text a reader sees. */
export interface WikilinkRef {
  target: WikilinkTarget;
  display: string;
}

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

const WIKILINK_TOKEN_RE = /\[\[([^[\]\n]+)\]\]/g;

/**
 * Parses the inner content of a `[[…]]` token. Mirrors the server's
 * `classify_wikilink`, including the rule that separates an address from prose:
 * `task:ATL-80` is a link, `task: rewrite the parser` is a title.
 */
export function parseWikilinkInner(inner: string): WikilinkRef {
  const pipe = inner.indexOf('|');
  const left = (pipe === -1 ? inner : inner.slice(0, pipe)).trim();
  const right = pipe === -1 ? null : inner.slice(pipe + 1).trim();

  const typed = typedTarget(left);
  if (typed !== null) {
    return { target: typed.target, display: right ?? typed.address };
  }

  if (right !== null && UUID_RE.test(left)) {
    return { target: { kind: 'document', id: left }, display: right };
  }

  return { target: { kind: 'title' }, display: inner.trim() };
}

function typedTarget(left: string): { target: WikilinkTarget; address: string } | null {
  const colon = left.indexOf(':');
  if (colon === -1) return null;

  const kind = left.slice(0, colon).toLowerCase();
  const rest = left.slice(colon + 1);

  if (/^\s/.test(rest)) return null;

  const address = rest.trimEnd();
  if (address.length === 0) return null;

  switch (kind) {
    // Readable ids are stored and matched uppercase, so a hand-typed `atl-80`
    // has to be normalized here to resolve.
    case 'task':
      return { target: { kind: 'task', readableId: address.toUpperCase() }, address };
    case 'note':
      return { target: { kind: 'note', slug: address }, address };
    case 'file':
      return { target: { kind: 'file', fileName: address }, address };
    default:
      return null;
  }
}

/**
 * The key under which a link's live title is cached, or `null` when the target
 * carries no title to resolve.
 *
 * Attachments are named by the file name already in the text, and a title-only
 * link is its own title, so neither has anything to look up.
 */
export function wikilinkTitleKey(ref: WikilinkRef): string | null {
  switch (ref.target.kind) {
    case 'task':
      return `task:${ref.target.readableId}`;
    case 'note':
      return `note:${ref.target.slug}`;
    case 'document':
      return ref.target.id;
    default:
      return null;
  }
}

/**
 * The text to render for a link: the target's live title when one has been
 * resolved, otherwise the display half written in the markdown.
 *
 * This is what makes a rename show up in every inbound link without rewriting
 * any stored text.
 */
export function wikilinkDisplay(ref: WikilinkRef, titles: Record<string, string>): string {
  const key = wikilinkTitleKey(ref);
  return (key !== null ? titles[key] : undefined) ?? ref.display;
}

/** Collects the unique title-resolution keys of every link in markdown. */
export function collectWikilinkTitleKeys(markdown: string): string[] {
  const keys = new Set<string>();
  WIKILINK_TOKEN_RE.lastIndex = 0;

  for (let m = WIKILINK_TOKEN_RE.exec(markdown); m !== null; m = WIKILINK_TOKEN_RE.exec(markdown)) {
    const inner = m[1];
    if (inner === undefined) continue;

    const key = wikilinkTitleKey(parseWikilinkInner(inner));
    if (key !== null) keys.add(key);
  }

  return [...keys];
}

/**
 * Serializes a reference back to wikilink markdown.
 *
 * The display half is omitted when it would only repeat the address, which is
 * what keeps a hand-written `[[task:ATL-80]]` from growing a redundant tail
 * every time it round-trips through the editor.
 */
export function formatWikilink(ref: WikilinkRef): string {
  const address = wikilinkAddress(ref);
  if (address === null) return `[[${ref.display}]]`;

  return address.bare === ref.display ? `[[${address.written}]]` : `[[${address.written}|${ref.display}]]`;
}

/**
 * The address as it is written in the markdown, alongside the bare identifier
 * inside it — a typed link parsed without a display half uses that identifier
 * as its display, and re-serializing must not append it a second time.
 */
function wikilinkAddress(ref: WikilinkRef): { written: string; bare: string } | null {
  switch (ref.target.kind) {
    case 'task':
      return { written: `task:${ref.target.readableId}`, bare: ref.target.readableId };
    case 'note':
      return { written: `note:${ref.target.slug}`, bare: ref.target.slug };
    case 'file':
      return { written: `file:${ref.target.fileName}`, bare: ref.target.fileName };
    // The id-bound form has no typed prefix, so dropping its display half
    // would turn it back into a title link on the next parse.
    case 'document':
      return { written: ref.target.id, bare: `${ref.target.id}|` };
    default:
      return null;
  }
}

/**
 * Resolves a wikilink to an in-app route, or `null` when it does not address a
 * navigable resource.
 *
 * An attachment is downloaded from its owner's route rather than opened as a
 * page, so it has no destination here.
 */
export function wikilinkHref(ref: WikilinkRef): string | null {
  switch (ref.target.kind) {
    case 'task':
      return `/t/task/${ref.target.readableId}`;
    case 'note':
      return `/n/${ref.target.slug}`;
    case 'document':
      return `/n/${ref.target.id}`;
    case 'file':
      return null;
    default:
      return `/n/${slugify(ref.display)}`;
  }
}
