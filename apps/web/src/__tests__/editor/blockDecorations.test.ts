import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { ensureSyntaxTree } from '@codemirror/language';
import { languages } from '@codemirror/language-data';
import { EditorSelection, EditorState } from '@codemirror/state';
import { GFM } from '@lezer/markdown';
import { describe, expect, it } from 'vitest';
import { buildBlockDecorations } from '@/components/editor/livePreviewExtension';
import { findMathRanges, isBlockActive } from '@/lib/livePreview';

/**
 * The block-decoration walk skips descending into paragraphs and headings (they
 * cannot contain a table or fenced-code block). These tests guard that the
 * optimization still discovers every block widget — including one that sits right
 * next to skipped nodes — and that the reveal-on-active-block rule is preserved.
 */

const ctx = { titles: {}, onWikilinkClick: () => {} };

function parsed(doc: string, cursor = 0): EditorState {
  const state = EditorState.create({
    doc,
    selection: EditorSelection.cursor(cursor),
    extensions: [markdown({ base: markdownLanguage, extensions: [GFM], codeLanguages: languages })],
  });
  ensureSyntaxTree(state, state.doc.length, 5000);
  return state;
}

const TABLE = ['| a | b |', '| - | - |', '| 1 | 2 |'].join('\n');
const MERMAID = ['```mermaid', 'graph TD; A-->B;', '```'].join('\n');

describe('math block range discovery', () => {
  it('finds inactive block math as a block-level range', () => {
    const doc = ['intro', '', '$$', 'x + y', '$$', '', 'after'].join('\n');
    expect(findMathRanges(doc)).toEqual([{ kind: 'block', from: 7, to: 18, bodyFrom: 10, bodyTo: 16 }]);
  });

  it('reveals block math when the active line touches the block', () => {
    const doc = ['intro', '', '$$', 'x + y', '$$', '', 'after'].join('\n');
    const [range] = findMathRanges(doc);
    if (range === undefined) throw new Error('expected math range');

    const state = parsed(doc, range.bodyFrom);
    const firstLine = state.doc.lineAt(range.from).number;
    const lastLine = state.doc.lineAt(range.to).number;

    expect(isBlockActive(firstLine, lastLine, new Set([state.doc.lineAt(range.bodyFrom).number]))).toBe(true);
    expect(isBlockActive(firstLine, lastLine, new Set([state.doc.lines]))).toBe(false);
  });
});

describe('buildBlockDecorations', () => {
  it('renders a top-level table as a block widget when the cursor is elsewhere', () => {
    const doc = `${TABLE}\n\nafter`;
    const state = parsed(doc, doc.length);
    expect(buildBlockDecorations(state, true, ctx).size).toBe(1);
  });

  it('reveals the table (no widget) when the selection is inside it', () => {
    const state = parsed(`${TABLE}\n\nafter`, 2);
    expect(buildBlockDecorations(state, true, ctx).size).toBe(0);
  });

  it('still finds a table that follows a paragraph (paragraph descent skipped)', () => {
    const doc = `a long paragraph of prose with **bold** and [a](http://x) links\n\n${TABLE}`;
    const state = parsed(doc, 0);
    expect(buildBlockDecorations(state, true, ctx).size).toBe(1);
  });

  it('still finds a table that follows a heading (heading descent skipped)', () => {
    const doc = `# A heading with *emphasis*\n\n${TABLE}`;
    const state = parsed(doc, 0);
    expect(buildBlockDecorations(state, true, ctx).size).toBe(1);
  });

  it('renders a mermaid fenced block as a widget', () => {
    const doc = `intro\n\n${MERMAID}`;
    const state = parsed(doc, 0);
    expect(buildBlockDecorations(state, true, ctx).size).toBe(1);
  });

  it('renders all blocks in preview mode (reveal off) even at the cursor', () => {
    const state = parsed(`${TABLE}\n\n${MERMAID}`, 2);
    expect(buildBlockDecorations(state, false, ctx).size).toBe(2);
  });
});
