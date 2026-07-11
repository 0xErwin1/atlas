/**
 * Pure decision logic for the Obsidian-style "Live Preview" markdown editor.
 *
 * The editor keeps the markdown source as the document; syntax markers (e.g. the
 * `#` of a heading, the `**` of bold) are visually hidden and the content styled
 * UNLESS the cursor/selection touches the line they sit on, in which case the raw
 * markers are revealed so the user can edit them.
 *
 * This module isolates the "which markers are revealed vs hidden" decision from
 * the CodeMirror view layer so it can be unit-tested without a DOM. The
 * CodeMirror ViewPlugin walks the Lezer tree and the wikilink regex, builds the
 * marker list, and asks these helpers what to do.
 */

/** A line, identified by its 1-based number and its absolute character range. */
export interface LineRange {
  /** 1-based line number. */
  number: number;
  /** Absolute document offset of the first character of the line. */
  from: number;
  /** Absolute document offset of the line terminator (or doc end). */
  to: number;
}

/** A selection range as absolute document offsets (head/anchor order-agnostic). */
export interface SelectionRange {
  from: number;
  to: number;
}

/**
 * A syntax marker that is a candidate for hiding. `from`/`to` delimit the raw
 * marker text in the document; `line` is the 1-based number of the line the
 * marker belongs to (markers never straddle lines in the constructs handled in
 * v1 — a fenced-code line decoration is applied per line, not per marker).
 */
export interface MarkerRange {
  from: number;
  to: number;
  line: number;
}

/**
 * Computes the set of "active" line numbers: a line is active when any selection
 * range intersects it, including a zero-width cursor sitting anywhere on the line
 * (start, middle, or end). A cursor exactly at the boundary between two lines
 * (the line terminator offset) activates the line that owns that offset as its
 * `to`, matching CodeMirror's `lineAt` behavior where the line's range is
 * `[from, to]` inclusive of the position at `to`.
 *
 * Returns a Set of 1-based line numbers.
 */
export function computeActiveLines(lines: LineRange[], selections: SelectionRange[]): Set<number> {
  const active = new Set<number>();

  for (const sel of selections) {
    const lo = Math.min(sel.from, sel.to);
    const hi = Math.max(sel.from, sel.to);

    for (const line of lines) {
      const intersects = lo <= line.to && hi >= line.from;
      if (intersects) active.add(line.number);
    }
  }

  return active;
}

/**
 * Decides whether a single marker should be REVEALED (shown raw, editable) given
 * the active line set. A marker is revealed iff its line is active. Off-active
 * markers are hidden by the caller.
 */
export function isMarkerRevealed(marker: MarkerRange, activeLines: Set<number>): boolean {
  return activeLines.has(marker.line);
}

/** Whether a GFM task marker (`[ ]`, `[x]`, `[X]`) is in the checked state. */
export function taskMarkerChecked(markerText: string): boolean {
  return /\[[xX]\]/.test(markerText);
}

/**
 * Extracts the language label from a fenced-code info string (the text after the
 * opening ```` ``` ````). Returns the first whitespace-delimited token, or null
 * when there is no language.
 */
export function fenceLanguage(infoText: string): string | null {
  const first = infoText.trim().split(/\s+/)[0] ?? '';
  return first.length > 0 ? first : null;
}

const IMAGE_RE = /^!\[([^\]]*)\]\(([^)\s]+)(?:\s+"[^"]*")?\)$/;

/**
 * Parses a markdown image `![alt](url)` (optionally with a `"title"`) into its alt
 * text and url. Returns null when the text is not a single complete image.
 */
export function parseImage(src: string): { alt: string; url: string } | null {
  const m = IMAGE_RE.exec(src.trim());
  if (m === null) return null;
  return { alt: m[1] ?? '', url: m[2] ?? '' };
}

export type MathKind = 'inline' | 'block';

export interface MathRange {
  kind: MathKind;
  from: number;
  to: number;
  bodyFrom: number;
  bodyTo: number;
}

export interface ExcludedRange {
  from: number;
  to: number;
}

export interface WikilinkRange {
  from: number;
  to: number;
  inner: string;
}

/** A run of inline markdown produced by {@link tokenizeInline}. */
export type InlineToken =
  | { type: 'text'; value: string }
  | { type: 'code'; value: string }
  | { type: 'strong'; value: string }
  | { type: 'em'; value: string }
  | { type: 'strike'; value: string }
  | { type: 'link'; value: string; url: string }
  | { type: 'wikilink'; value: string }
  | { type: 'math'; value: string };

const INLINE_RE =
  /(`[^`]+`)|(\[\[[^\]]+\]\])|(\[[^\]]+\]\([^)]+\))|(\$[^$\n]+\$)|(\*\*[^*]+\*\*|__[^_]+__)|(~~[^~]+~~)|(\*[^*]+\*|_[^_]+_)/;

const WIKILINK_RE = /\[\[([^\]\n[]+)\]\]/g;

/**
 * Splits a line of inline markdown into typed tokens (text, code, bold, italic,
 * strikethrough, link, wikilink). Used to render table cells as formatted content
 * instead of raw markdown. Single-level only: a mark cannot contain another mark
 * (enough for table cells); anything unrecognised stays as text.
 */
export function tokenizeInline(text: string): InlineToken[] {
  const tokens: InlineToken[] = [];
  const re = new RegExp(INLINE_RE.source, 'g');
  let last = 0;

  for (let m = re.exec(text); m !== null; m = re.exec(text)) {
    if (m.index > last) tokens.push({ type: 'text', value: text.slice(last, m.index) });

    const [whole, code, wikilink, link, math, strong, strike, em] = m;

    if (code !== undefined) {
      tokens.push({ type: 'code', value: code.slice(1, -1) });
    } else if (wikilink !== undefined) {
      tokens.push({ type: 'wikilink', value: wikilink.slice(2, -2) });
    } else if (link !== undefined) {
      const lm = /^\[([^\]]+)\]\(([^)]+)\)$/.exec(link);
      tokens.push(
        lm !== null ? { type: 'link', value: lm[1] ?? '', url: lm[2] ?? '' } : { type: 'text', value: link },
      );
    } else if (math !== undefined) {
      const [range] = findMathRanges(math);
      tokens.push(
        range?.kind === 'inline' && range.from === 0 && range.to === math.length
          ? { type: 'math', value: math.slice(1, -1) }
          : { type: 'text', value: math },
      );
    } else if (strong !== undefined) {
      tokens.push({ type: 'strong', value: strong.slice(2, -2) });
    } else if (strike !== undefined) {
      tokens.push({ type: 'strike', value: strike.slice(2, -2) });
    } else if (em !== undefined) {
      tokens.push({ type: 'em', value: em.slice(1, -1) });
    }

    last = m.index + (whole?.length ?? 0);
  }

  if (last < text.length) tokens.push({ type: 'text', value: text.slice(last) });

  return tokens;
}

function isEscaped(text: string, pos: number): boolean {
  let count = 0;
  for (let i = pos - 1; i >= 0 && text[i] === '\\'; i -= 1) count += 1;
  return count % 2 === 1;
}

function overlapsExcluded(from: number, to: number, excluded: ExcludedRange[]): boolean {
  return excluded.some((range) => from < range.to && to > range.from);
}

function containsExcluded(pos: number, excluded: ExcludedRange[]): boolean {
  return excluded.some((range) => pos >= range.from && pos < range.to);
}

function lineRanges(doc: string): Array<{ from: number; to: number; text: string }> {
  const lines: Array<{ from: number; to: number; text: string }> = [];
  let from = 0;
  for (const text of doc.split('\n')) {
    const to = from + text.length;
    lines.push({ from, to, text });
    from = to + 1;
  }
  return lines;
}

function fencedCodeRanges(doc: string): ExcludedRange[] {
  const ranges: ExcludedRange[] = [];
  const lines = lineRanges(doc);
  let open: { from: number; marker: string } | null = null;

  for (const line of lines) {
    const opening = /^( {0,3})(`{3,}|~{3,})/.exec(line.text);
    if (open === null) {
      if (opening !== null) open = { from: line.from, marker: opening[2] ?? '' };
      continue;
    }

    const closing = /^( {0,3})(`{3,}|~{3,})\s*$/.exec(line.text);
    if (closing !== null) {
      const marker = closing[2] ?? '';
      if (marker[0] === open.marker[0] && marker.length >= open.marker.length) {
        ranges.push({ from: open.from, to: line.to });
        open = null;
      }
    }
  }

  if (open !== null) ranges.push({ from: open.from, to: doc.length });
  return ranges;
}

function inlineCodeRanges(doc: string, excluded: ExcludedRange[]): ExcludedRange[] {
  const ranges: ExcludedRange[] = [];

  for (const line of lineRanges(doc)) {
    let open: { from: number; length: number } | null = null;
    for (let pos = line.from; pos < line.to; pos += 1) {
      if (doc[pos] !== '`' || isEscaped(doc, pos) || containsExcluded(pos, excluded)) continue;

      let runEnd = pos + 1;
      while (runEnd < line.to && doc[runEnd] === '`') runEnd += 1;
      const length = runEnd - pos;

      if (open === null) {
        open = { from: pos, length };
      } else if (length === open.length) {
        ranges.push({ from: open.from, to: runEnd });
        open = null;
      }

      pos = runEnd - 1;
    }
  }

  return ranges;
}

function findBlockMathRanges(doc: string, excluded: ExcludedRange[]): MathRange[] {
  const ranges: MathRange[] = [];
  const lines = lineRanges(doc);
  let open: { from: number; to: number } | null = null;

  for (const line of lines) {
    if (overlapsExcluded(line.from, line.to, excluded)) continue;

    const trimmed = line.text.trim();
    const indent = line.text.search(/\S/);
    if (indent < 0) continue;
    const trimmedFrom = line.from + indent;

    if (open !== null) {
      if (trimmed === '$$') {
        ranges.push({
          kind: 'block',
          from: open.from,
          to: trimmedFrom + 2,
          bodyFrom: open.to + 1,
          bodyTo: trimmedFrom,
        });
        open = null;
      }
      continue;
    }

    if (!trimmed.startsWith('$$')) continue;
    const afterOpen = trimmed.slice(2);
    if (afterOpen.length === 0) {
      open = { from: trimmedFrom, to: trimmedFrom + 2 };
      continue;
    }

    if (trimmed.endsWith('$$') && trimmed.length > 4) {
      const closeFrom = trimmedFrom + trimmed.length - 2;
      const bodyFrom = trimmedFrom + 2;
      if (doc.slice(bodyFrom, closeFrom).trim().length > 0) {
        ranges.push({ kind: 'block', from: trimmedFrom, to: closeFrom + 2, bodyFrom, bodyTo: closeFrom });
      }
    }
  }

  return ranges;
}

function canOpenInlineMath(doc: string, pos: number): boolean {
  if (doc[pos] !== '$' || isEscaped(doc, pos)) return false;
  if (doc[pos + 1] === '$' || doc[pos - 1] === '$') return false;
  const next = doc[pos + 1];
  if (next === undefined || /\s|\d/.test(next)) return false;
  return true;
}

function canCloseInlineMath(doc: string, pos: number): boolean {
  if (doc[pos] !== '$' || isEscaped(doc, pos)) return false;
  if (doc[pos + 1] === '$' || doc[pos - 1] === '$') return false;
  const prev = doc[pos - 1];
  if (prev === undefined || /\s/.test(prev)) return false;
  return true;
}

function findInlineMathRanges(doc: string, excluded: ExcludedRange[]): MathRange[] {
  const ranges: MathRange[] = [];

  for (const line of lineRanges(doc)) {
    let open: number | null = null;
    for (let pos = line.from; pos < line.to; pos += 1) {
      if (doc[pos] !== '$' || containsExcluded(pos, excluded)) continue;

      if (open === null) {
        if (canOpenInlineMath(doc, pos)) open = pos;
        continue;
      }

      if (!canCloseInlineMath(doc, pos)) continue;
      const bodyFrom = open + 1;
      const bodyTo = pos;
      if (!overlapsExcluded(open, pos + 1, excluded) && doc.slice(bodyFrom, bodyTo).trim().length > 0) {
        ranges.push({ kind: 'inline', from: open, to: pos + 1, bodyFrom, bodyTo });
      }
      open = null;
    }
  }

  return ranges;
}

export function findMathRanges(doc: string, excluded: ExcludedRange[] = []): MathRange[] {
  const baseExcluded = [...excluded, ...fencedCodeRanges(doc)];
  const blockRanges = findBlockMathRanges(doc, baseExcluded);
  const allExcluded = [...baseExcluded, ...blockRanges, ...inlineCodeRanges(doc, baseExcluded)];
  return [...blockRanges, ...findInlineMathRanges(doc, allExcluded)].sort((a, b) => a.from - b.from);
}

export function findWikilinkRanges(doc: string, excluded: ExcludedRange[] = []): WikilinkRange[] {
  const baseExcluded = [...excluded, ...fencedCodeRanges(doc)];
  const allExcluded = [...baseExcluded, ...inlineCodeRanges(doc, baseExcluded)];
  const ranges: WikilinkRange[] = [];

  WIKILINK_RE.lastIndex = 0;
  for (let m = WIKILINK_RE.exec(doc); m !== null; m = WIKILINK_RE.exec(doc)) {
    const inner = m[1];
    if (inner === undefined) continue;

    const from = m.index;
    const to = from + m[0].length;
    if (overlapsExcluded(from, to, allExcluded)) continue;

    ranges.push({ from, to, inner });
  }

  return ranges;
}

/** Per-column horizontal alignment from a GFM table delimiter row. */
export type ColumnAlign = 'left' | 'center' | 'right' | null;

export interface ParsedTable {
  headers: string[];
  aligns: ColumnAlign[];
  rows: string[][];
}

function splitTableRow(line: string): string[] {
  let s = line.trim();
  if (s.startsWith('|')) s = s.slice(1);
  if (s.endsWith('|')) s = s.slice(0, -1);

  const cells: string[] = [];
  let cellFrom = 0;
  let inWikilink = false;

  for (let pos = 0; pos < s.length; pos += 1) {
    if (!inWikilink && s.startsWith('[[', pos) && s.indexOf(']]', pos + 2) !== -1) {
      inWikilink = true;
      pos += 1;
    } else if (inWikilink && s.startsWith(']]', pos)) {
      inWikilink = false;
      pos += 1;
    } else if (!inWikilink && s[pos] === '|') {
      cells.push(s.slice(cellFrom, pos).trim());
      cellFrom = pos + 1;
    }
  }

  cells.push(s.slice(cellFrom).trim());
  return cells;
}

/**
 * Parses a GFM table block (header row, `---` delimiter row, then body rows) into
 * its cells and per-column alignment. Returns null when the text is not a valid
 * table (missing or malformed delimiter row).
 */
export function parseTable(src: string): ParsedTable | null {
  const lines = src
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l.length > 0);

  if (lines.length < 2) return null;

  const headers = splitTableRow(lines[0] ?? '');
  const delim = splitTableRow(lines[1] ?? '');

  const isDelim = delim.length > 0 && delim.every((c) => /^:?-+:?$/.test(c));
  if (!isDelim) return null;

  const aligns: ColumnAlign[] = delim.map((c) => {
    const left = c.startsWith(':');
    const right = c.endsWith(':');
    if (left && right) return 'center';
    if (right) return 'right';
    if (left) return 'left';
    return null;
  });

  const rows = lines.slice(2).map(splitTableRow);

  return { headers, aligns, rows };
}

/**
 * Whether a block construct (table, fenced diagram) should be revealed as raw
 * markdown for editing: true when the selection touches any line the block spans,
 * so clicking into it turns the rendered widget back into editable source.
 */
export function isBlockActive(firstLine: number, lastLine: number, activeLines: Set<number>): boolean {
  for (let n = firstLine; n <= lastLine; n += 1) {
    if (activeLines.has(n)) return true;
  }
  return false;
}

/**
 * Partitions a marker list into the ranges to HIDE (replace/collapse) and the
 * ranges to REVEAL (leave raw) for the current selection. This is the single
 * decision the live-preview ViewPlugin consumes to build its DecorationSet.
 */
export function partitionMarkers(
  markers: MarkerRange[],
  lines: LineRange[],
  selections: SelectionRange[],
): { hidden: MarkerRange[]; revealed: MarkerRange[] } {
  const activeLines = computeActiveLines(lines, selections);

  const hidden: MarkerRange[] = [];
  const revealed: MarkerRange[] = [];

  for (const marker of markers) {
    if (isMarkerRevealed(marker, activeLines)) {
      revealed.push(marker);
    } else {
      hidden.push(marker);
    }
  }

  return { hidden, revealed };
}
