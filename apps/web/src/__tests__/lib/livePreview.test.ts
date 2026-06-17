import { describe, expect, it } from 'vitest';
import {
  computeActiveLines,
  isMarkerRevealed,
  type LineRange,
  type MarkerRange,
  partitionMarkers,
  type SelectionRange,
} from '@/lib/livePreview';

/**
 * Builds line ranges from a multi-line string, computing absolute offsets the
 * same way CodeMirror's Text does: each line's `to` is the offset of its newline
 * (or the doc end for the last line), `from` of the next line is `to + 1`.
 */
function linesOf(doc: string): LineRange[] {
  const out: LineRange[] = [];
  let from = 0;
  let number = 1;

  for (const text of doc.split('\n')) {
    const to = from + text.length;
    out.push({ number, from, to });
    from = to + 1;
    number += 1;
  }

  return out;
}

/** Indexed access into a known-populated fixture array, asserting presence. */
function at<T>(arr: T[], i: number): T {
  const v = arr[i];
  if (v === undefined) throw new Error(`fixture index ${i} out of range`);
  return v;
}

describe('computeActiveLines', () => {
  const doc = '# Title\nbody text\n**bold**';
  const lines = linesOf(doc);

  it('marks the line a zero-width cursor sits on as active', () => {
    const cursor: SelectionRange = { from: 2, to: 2 };
    expect(computeActiveLines(lines, [cursor])).toEqual(new Set([1]));
  });

  it('marks a line active when the cursor sits at its end boundary', () => {
    const endOfLine1 = at(lines, 0).to;
    const cursor: SelectionRange = { from: endOfLine1, to: endOfLine1 };
    expect(computeActiveLines(lines, [cursor]).has(1)).toBe(true);
  });

  it('marks every line a multi-line selection spans as active', () => {
    const sel: SelectionRange = { from: 2, to: at(lines, 2).from + 2 };
    expect(computeActiveLines(lines, [sel])).toEqual(new Set([1, 2, 3]));
  });

  it('handles reversed selections (anchor after head)', () => {
    const sel: SelectionRange = { from: at(lines, 2).from + 2, to: 2 };
    expect(computeActiveLines(lines, [sel])).toEqual(new Set([1, 2, 3]));
  });

  it('unions multiple disjoint selection ranges', () => {
    const a: SelectionRange = { from: 1, to: 1 };
    const c: SelectionRange = { from: at(lines, 2).from, to: at(lines, 2).from };
    expect(computeActiveLines(lines, [a, c])).toEqual(new Set([1, 3]));
  });

  it('returns an empty set when there are no selections', () => {
    expect(computeActiveLines(lines, [])).toEqual(new Set());
  });
});

describe('isMarkerRevealed', () => {
  it('reveals a marker on an active line', () => {
    const marker: MarkerRange = { from: 0, to: 1, line: 1 };
    expect(isMarkerRevealed(marker, new Set([1]))).toBe(true);
  });

  it('hides a marker whose line is not active', () => {
    const marker: MarkerRange = { from: 0, to: 1, line: 2 };
    expect(isMarkerRevealed(marker, new Set([1]))).toBe(false);
  });
});

describe('partitionMarkers', () => {
  // doc:  "### Heading"  (line 1)  +  "a **b** c"  (line 3)
  const doc = '### Heading\n\na **b** c';
  const lines = linesOf(doc);

  // HeaderMark "### " on line 1, and the two EmphasisMark "**" on line 3.
  const headerMark: MarkerRange = { from: 0, to: 4, line: 1 };
  const openBold: MarkerRange = { from: at(lines, 2).from + 2, to: at(lines, 2).from + 4, line: 3 };
  const closeBold: MarkerRange = { from: at(lines, 2).from + 5, to: at(lines, 2).from + 7, line: 3 };
  const markers = [headerMark, openBold, closeBold];

  it('hides every marker when the cursor is on an unrelated line', () => {
    const cursor: SelectionRange = { from: at(lines, 1).from, to: at(lines, 1).from };
    const { hidden, revealed } = partitionMarkers(markers, lines, [cursor]);

    expect(revealed).toEqual([]);
    expect(hidden).toEqual(markers);
  });

  it('reveals only the markers on the cursor line, hides the rest', () => {
    const cursorOnHeading: SelectionRange = { from: 1, to: 1 };
    const { hidden, revealed } = partitionMarkers(markers, lines, [cursorOnHeading]);

    expect(revealed).toEqual([headerMark]);
    expect(hidden).toEqual([openBold, closeBold]);
  });

  it('reveals the inline marks when the cursor enters the bold word', () => {
    const cursorInBold: SelectionRange = { from: at(lines, 2).from + 4, to: at(lines, 2).from + 4 };
    const { hidden, revealed } = partitionMarkers(markers, lines, [cursorInBold]);

    expect(revealed).toEqual([openBold, closeBold]);
    expect(hidden).toEqual([headerMark]);
  });
});
