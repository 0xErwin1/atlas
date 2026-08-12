import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { ensureSyntaxTree, syntaxTree } from '@codemirror/language';
import { EditorState } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import { GFM } from '@lezer/markdown';
import { afterEach, describe, expect, it } from 'vitest';
import { paragraphUnwrapChanges, unwrapParagraphs } from '@/components/editor/unwrapParagraphs';

/**
 * The unwrap command rewrites the markdown source, so every test asserts on the
 * resulting document text: the point is what gets persisted, not what is drawn.
 */

const views: EditorView[] = [];

function stateFor(doc: string): { state: EditorState; tree: ReturnType<typeof syntaxTree> } {
  const state = EditorState.create({
    doc,
    extensions: [markdown({ base: markdownLanguage, extensions: [GFM] })],
  });
  const tree = ensureSyntaxTree(state, state.doc.length, 5000) ?? syntaxTree(state);
  return { state, tree };
}

function unwrapped(doc: string): string {
  const parent = document.createElement('div');
  document.body.appendChild(parent);
  const { state } = stateFor(doc);
  const view = new EditorView({ state, parent });
  views.push(view);

  unwrapParagraphs(view);
  return view.state.doc.toString();
}

afterEach(() => {
  for (const view of views.splice(0)) {
    const parent = view.dom.parentElement;
    view.destroy();
    parent?.remove();
  }
});

describe('paragraphUnwrapChanges', () => {
  it('produces one change per soft break', () => {
    const { state, tree } = stateFor('one\ntwo\nthree');
    expect(paragraphUnwrapChanges(state, tree)).toHaveLength(2);
  });

  it('produces nothing for an already-unwrapped document', () => {
    const { state, tree } = stateFor('one long paragraph\n\nand another');
    expect(paragraphUnwrapChanges(state, tree)).toEqual([]);
  });
});

describe('unwrapParagraphs', () => {
  it('joins a hard-wrapped paragraph into one line', () => {
    expect(unwrapped('Un lenguaje interpretado tiene\nque decidir qué ejecuta.')).toBe(
      'Un lenguaje interpretado tiene que decidir qué ejecuta.',
    );
  });

  it('keeps paragraphs separated by a blank line apart', () => {
    expect(unwrapped('First\nparagraph.\n\nSecond\nparagraph.')).toBe(
      'First paragraph.\n\nSecond paragraph.',
    );
  });

  it('drops the blockquote markers of the continuation lines', () => {
    expect(unwrapped('> Guía didáctica, escrita\n> desde cero.')).toBe(
      '> Guía didáctica, escrita desde cero.',
    );
  });

  it('drops the indentation of a wrapped list item', () => {
    expect(unwrapped('- una opción larga\n  que sigue acá')).toBe('- una opción larga que sigue acá');
  });

  it('preserves a two-space hard break', () => {
    expect(unwrapped('Line one.  \nLine two.')).toBe('Line one.  \nLine two.');
  });

  it('preserves a backslash hard break', () => {
    expect(unwrapped('Line one.\\\nLine two.')).toBe('Line one.\\\nLine two.');
  });

  it('leaves a fenced code block untouched', () => {
    const doc = '```\nBinary(+)\n  / \\\n```';
    expect(unwrapped(doc)).toBe(doc);
  });

  it('leaves a table untouched', () => {
    const doc = '| a | b |\n| - | - |\n| 1 | 2 |';
    expect(unwrapped(doc)).toBe(doc);
  });

  it('leaves headings and horizontal rules untouched', () => {
    const doc = '# Title\n\n---\n\n## Section';
    expect(unwrapped(doc)).toBe(doc);
  });

  it('reports whether it changed anything', () => {
    const parent = document.createElement('div');
    document.body.appendChild(parent);
    const view = new EditorView({ state: stateFor('already one line').state, parent });
    views.push(view);

    expect(unwrapParagraphs(view)).toBe(false);
  });
});
