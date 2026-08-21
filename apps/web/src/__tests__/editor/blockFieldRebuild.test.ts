import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { ensureSyntaxTree, syntaxTree } from '@codemirror/language';
import { languages } from '@codemirror/language-data';
import { EditorSelection, EditorState } from '@codemirror/state';
import { type DecorationSet, EditorView } from '@codemirror/view';
import { GFM } from '@lezer/markdown';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { livePreview } from '@/components/editor/livePreviewExtension';
import { parseTable } from '@/lib/livePreview';

vi.mock('@/lib/livePreview', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/lib/livePreview')>();
  return { ...actual, parseTable: vi.fn(actual.parseTable) };
});

vi.mock('@codemirror/language', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@codemirror/language')>();
  return { ...actual, ensureSyntaxTree: vi.fn(actual.ensureSyntaxTree) };
});

const parseTableSpy = vi.mocked(parseTable);
const ensureSyntaxTreeSpy = vi.mocked(ensureSyntaxTree);

const TABLE_A = ['| a | b |', '| - | - |', '| 1 | 2 |'].join('\n');
const TABLE_B = ['| c | d |', '| - | - |', '| 3 | 4 |'].join('\n');
const PROSE = 'plain prose line one\nplain prose line two';
const DOC = [PROSE, '', TABLE_A, '', 'between the tables', '', TABLE_B, '', 'tail prose'].join('\n');

function stateFor(doc: string, cursor = 0): EditorState {
  return EditorState.create({
    doc,
    selection: EditorSelection.cursor(cursor),
    extensions: [
      markdown({ base: markdownLanguage, extensions: [GFM], codeLanguages: languages }),
      livePreview({ onWikilinkClick: () => {} }, { reveal: true }),
    ],
  });
}

/** The block StateField's decoration set, which is the only plain set in the facet. */
function blockDecorations(state: EditorState): DecorationSet {
  const sets = state
    .facet(EditorView.decorations)
    .filter((value): value is DecorationSet => typeof value !== 'function');
  const set = sets[0];
  if (set === undefined) throw new Error('block decorations missing');
  return set;
}

/** Calls of ensureSyntaxTree that span the whole document, which only the block field makes. */
function wholeDocEnsureCalls(docLength: number): number {
  return ensureSyntaxTreeSpy.mock.calls.filter(([, upto]) => upto === docLength).length;
}

beforeEach(() => {
  parseTableSpy.mockClear();
  ensureSyntaxTreeSpy.mockClear();
});

describe('block decoration field rebuild policy', () => {
  it('builds every table on create', () => {
    const state = stateFor(DOC);

    expect(parseTableSpy).toHaveBeenCalledTimes(2);
    expect(blockDecorations(state).size).toBe(2);
  });

  it('skips the rebuild on a selection move that stays in plain text', () => {
    const state = stateFor(DOC);
    parseTableSpy.mockClear();
    ensureSyntaxTreeSpy.mockClear();

    const moved = state.update({ selection: EditorSelection.cursor(5) }).state;

    expect(parseTableSpy).not.toHaveBeenCalled();
    expect(wholeDocEnsureCalls(DOC.length)).toBe(0);
    expect(blockDecorations(moved).size).toBe(2);
  });

  it('rebuilds when the selection enters a table, and again when it leaves', () => {
    const state = stateFor(DOC);
    parseTableSpy.mockClear();

    const inside = state.update({ selection: EditorSelection.cursor(DOC.indexOf(TABLE_A) + 2) }).state;

    expect(parseTableSpy).toHaveBeenCalledTimes(1);
    expect(blockDecorations(inside).size).toBe(1);

    const withinSameTable = inside.update({
      selection: EditorSelection.cursor(DOC.indexOf(TABLE_A) + 4),
    }).state;

    expect(parseTableSpy).toHaveBeenCalledTimes(1);
    expect(blockDecorations(withinSameTable).size).toBe(1);

    const outside = withinSameTable.update({ selection: EditorSelection.cursor(DOC.indexOf('tail')) }).state;

    expect(parseTableSpy).toHaveBeenCalledTimes(3);
    expect(blockDecorations(outside).size).toBe(2);
  });

  it('rebuilds on a document change without forcing a whole-document parse', () => {
    const state = stateFor(DOC);
    parseTableSpy.mockClear();
    ensureSyntaxTreeSpy.mockClear();

    const edited = state.update({ changes: { from: 0, insert: 'x' } }).state;

    expect(parseTableSpy).toHaveBeenCalledTimes(2);
    expect(wholeDocEnsureCalls(edited.doc.length)).toBe(0);
    expect(blockDecorations(edited).size).toBe(2);
  });
});

describe('block decoration field on background parse progress', () => {
  const views: EditorView[] = [];

  afterEach(() => {
    for (const view of views.splice(0)) {
      const parent = view.dom.parentElement;
      view.destroy();
      parent?.remove();
    }
  });

  it('rebuilds from the parser-delivered tree instead of forcing the parse itself', async () => {
    const filler = Array.from(
      { length: 80 },
      (_, i) => `Paragraph ${i} with some words to pad the document.`,
    ).join('\n\n');
    const doc = `${filler}\n\n${TABLE_A}\n`;
    expect(doc.length).toBeGreaterThan(3000);

    const state = stateFor(doc);
    expect(syntaxTree(state).length).toBeLessThan(doc.length);

    const parent = document.createElement('div');
    document.body.appendChild(parent);
    const view = new EditorView({ state, parent });
    views.push(view);

    parseTableSpy.mockClear();
    ensureSyntaxTreeSpy.mockClear();

    for (let i = 0; i < 50 && syntaxTree(view.state).length < doc.length; i += 1) {
      await new Promise((resolve) => setTimeout(resolve, 20));
    }

    expect(syntaxTree(view.state).length).toBe(doc.length);
    expect(parseTableSpy).toHaveBeenCalled();
    expect(wholeDocEnsureCalls(doc.length)).toBe(0);
    expect(blockDecorations(view.state).size).toBe(1);
  });
});
