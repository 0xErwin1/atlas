import { EditorSelection, EditorState } from '@codemirror/state';
import { describe, expect, it } from 'vitest';
import { activeLinesFromSelection } from '@/components/editor/livePreviewExtension';
import { computeActiveLines, type LineRange } from '@/lib/livePreview';

/**
 * `activeLinesFromSelection` derives the revealed-line set directly from the
 * selection (O(selection) via `lineAt`) instead of scanning every document line.
 * These tests pin it to the pure `computeActiveLines` reference — which scans all
 * lines — so the optimization can never silently diverge from the intersection
 * rule the live-preview reveal depends on.
 */

function allLineRanges(state: EditorState): LineRange[] {
  const out: LineRange[] = [];
  for (let n = 1; n <= state.doc.lines; n += 1) {
    const line = state.doc.line(n);
    out.push({ number: line.number, from: line.from, to: line.to });
  }
  return out;
}

function reference(state: EditorState): Set<number> {
  const sels = state.selection.ranges.map((r) => ({ from: r.from, to: r.to }));
  return computeActiveLines(allLineRanges(state), sels);
}

const DOC = ['# Title', '', 'first paragraph', 'second line', '', 'last one'].join('\n');

function stateWith(ranges: ReturnType<typeof EditorSelection.range>[]): EditorState {
  return EditorState.create({ doc: DOC, selection: EditorSelection.create(ranges) });
}

describe('activeLinesFromSelection', () => {
  it('returns an empty set when reveal is off, regardless of selection', () => {
    const state = stateWith([EditorSelection.cursor(3)]);
    expect(activeLinesFromSelection(state, false).size).toBe(0);
  });

  it('matches the reference for a cursor in the middle of a line', () => {
    const state = stateWith([EditorSelection.cursor(10)]);
    expect(activeLinesFromSelection(state, true)).toEqual(reference(state));
  });

  it('matches the reference for a cursor at a line boundary', () => {
    const lineStart = DOC.indexOf('first paragraph');
    const state = stateWith([EditorSelection.cursor(lineStart)]);
    expect(activeLinesFromSelection(state, true)).toEqual(reference(state));
  });

  it('matches the reference for a multi-line selection', () => {
    const from = DOC.indexOf('first');
    const to = DOC.indexOf('second line') + 3;
    const state = stateWith([EditorSelection.range(from, to)]);
    expect(activeLinesFromSelection(state, true)).toEqual(reference(state));
  });

  it('matches the reference for a reversed (head before anchor) selection', () => {
    const from = DOC.indexOf('second line') + 3;
    const to = DOC.indexOf('first');
    const state = stateWith([EditorSelection.range(from, to)]);
    expect(activeLinesFromSelection(state, true)).toEqual(reference(state));
  });

  it('matches the reference for multiple cursors', () => {
    const state = stateWith([EditorSelection.cursor(2), EditorSelection.cursor(DOC.length)]);
    expect(activeLinesFromSelection(state, true)).toEqual(reference(state));
  });

  it('matches the reference at the document end', () => {
    const state = stateWith([EditorSelection.cursor(DOC.length)]);
    expect(activeLinesFromSelection(state, true)).toEqual(reference(state));
  });
});
