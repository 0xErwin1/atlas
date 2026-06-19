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
