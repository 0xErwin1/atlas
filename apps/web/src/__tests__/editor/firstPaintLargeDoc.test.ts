import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { ensureSyntaxTree, syntaxTree, syntaxTreeAvailable } from '@codemirror/language';
import { languages } from '@codemirror/language-data';
import { EditorSelection, EditorState } from '@codemirror/state';
import type { Decoration, DecorationSet, EditorView } from '@codemirror/view';
import { GFM } from '@lezer/markdown';
import { describe, expect, it } from 'vitest';
import { buildBlockDecorations, buildDecorations } from '@/components/editor/livePreviewExtension';

/**
 * Regression guard for the large-document first-paint bug: the live-preview
 * ViewPlugin used to build its decorations from `syntaxTree(view.state)`, which on
 * initial load only spans CodeMirror's ~3 KB init parse (`Work.InitViewport`).
 * A larger document (headings/tables past that boundary) therefore rendered as
 * raw, unstyled markdown until a background-parse transaction happened to arrive.
 * `ensureSyntaxTree` advances the parse but does NOT refresh the state field, so
 * the fix reads the tree ensureSyntaxTree RETURNS instead.
 */

const INIT_PARSE_CAP = 3000; // CodeMirror Work.InitViewport

function markdownState(doc: string): EditorState {
  return EditorState.create({
    doc,
    selection: EditorSelection.cursor(0),
    extensions: [markdown({ base: markdownLanguage, extensions: [GFM], codeLanguages: languages })],
  });
}

function docWithHeadings(count: number): string {
  const parts: string[] = [];
  for (let i = 0; i < count; i += 1) {
    parts.push(`# Encabezado ${i} con acentós áéíóú`);
    parts.push('');
    parts.push(`Párrafo ${i} con **negrita**, *cursiva* y \`código\` para ocupar espacio.`);
    parts.push('');
  }
  return parts.join('\n');
}

/** A view stand-in with a bounded viewport past the 3 KB init cap. jsdom cannot
 * produce a viewport larger than a few hundred characters, so this mock exposes
 * the past-3 KB region the real bug lived in. Only the fields buildDecorations
 * reads are provided. */
function viewWithViewport(state: EditorState, viewportTo: number): EditorView {
  return {
    state,
    viewport: { from: 0, to: viewportTo },
    visibleRanges: [{ from: 0, to: viewportTo }],
  } as unknown as EditorView;
}

/**
 * Drives the shared parse context forward until the tree spans `upto`,
 * independent of any single wall-clock budget. This mirrors what a real webview
 * achieves for a bounded viewport at first paint, but deterministically so the
 * assertion never depends on how much CPU a heavily-parallel test run grants a
 * fixed millisecond budget.
 */
function parseUpTo(state: EditorState, upto: number): void {
  let guard = 0;
  while (!syntaxTreeAvailable(state, upto) && guard < 100) {
    ensureSyntaxTree(state, upto, 1000);
    guard += 1;
  }
}

function headingsBefore(state: EditorState, offset: number): number {
  let n = 0;
  for (let i = 1; i <= state.doc.lines; i += 1) {
    const line = state.doc.line(i);
    if (line.from >= offset) break;
    if (line.text.startsWith('# ')) n += 1;
  }
  return n;
}

function countClass(set: DecorationSet, cls: string, docLength: number): number {
  let n = 0;
  set.between(0, docLength, (_from: number, _to: number, value: Decoration) => {
    if ((value.spec as { class?: string }).class === cls) n += 1;
  });
  return n;
}

describe('live-preview first paint on large documents', () => {
  it('confirms ensureSyntaxTree advances the parse without updating the state field', () => {
    const state = markdownState(docWithHeadings(400));

    expect(state.doc.length).toBeGreaterThan(10000);
    expect(syntaxTree(state).length).toBeLessThan(INIT_PARSE_CAP + 200);

    const returned = ensureSyntaxTree(state, 9000, 5000);
    expect(returned).not.toBeNull();
    expect(returned?.length ?? 0).toBeGreaterThanOrEqual(9000);

    // The reason reading syntaxTree(state) is wrong: it stays on the init tree.
    expect(syntaxTree(state).length).toBeLessThan(INIT_PARSE_CAP + 200);
  });

  it('decorates every heading across the viewport, not just the first ~3 KB', () => {
    const state = markdownState(docWithHeadings(400));
    const viewportTo = 8000; // a realistic bounded viewport, well past the 3 KB cap

    parseUpTo(state, viewportTo);

    // The state field tree stays on the short init parse: a build reading
    // syntaxTree(state) directly (the bug) would miss every heading past ~3 KB.
    expect(syntaxTree(state).length).toBeLessThan(INIT_PARSE_CAP + 200);

    const expected = headingsBefore(state, viewportTo);
    expect(expected).toBeGreaterThan(30); // many headings live beyond the init cap

    const built = buildDecorations(
      viewWithViewport(state, viewportTo),
      { onWikilinkClick: () => {} },
      true,
      {},
    );
    const headings = countClass(built.decorations, 'cm-atlas-h1', viewportTo);

    expect(headings).toBe(expected);
  });

  it('renders block widgets (tables) past the init-parse boundary', () => {
    const filler = docWithHeadings(120); // well over 3 KB of content
    expect(filler.length).toBeGreaterThan(INIT_PARSE_CAP);
    const table = ['| a | b |', '| - | - |', '| 1 | 2 |'].join('\n');
    const doc = `${filler}\n${table}\n`;

    const state = markdownState(doc);
    expect(syntaxTree(state).length).toBeLessThan(INIT_PARSE_CAP + 200);

    const ctx = { titles: {}, onWikilinkClick: () => {} };

    // Reading the stale state-field tree misses the far table.
    expect(buildBlockDecorations(state, true, ctx).size).toBe(0);

    // Handed a tree that spans the document, the table renders as a block widget.
    const fullTree = ensureSyntaxTree(state, state.doc.length, 5000) ?? syntaxTree(state);
    expect(buildBlockDecorations(state, true, ctx, fullTree).size).toBe(1);
  });
});
