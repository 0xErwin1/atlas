import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { ensureSyntaxTree } from '@codemirror/language';
import { EditorSelection, EditorState } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import { GFM } from '@lezer/markdown';
import { afterEach, describe, expect, it } from 'vitest';
import { livePreview } from '@/components/editor/livePreviewExtension';

/**
 * Reading mode renders a hard-wrapped markdown source the way CommonMark defines
 * it: a single newline inside a paragraph is a space, not a break, and a rule is
 * a rule rather than a rule plus its `---`. These tests assert on the rendered
 * DOM because the whole point is visual — the source is never touched.
 */

const views: EditorView[] = [];

function viewFor(doc: string, reveal: boolean, cursor = 0): EditorView {
  const parent = document.createElement('div');
  document.body.appendChild(parent);
  const state = EditorState.create({
    doc,
    selection: EditorSelection.cursor(cursor),
    extensions: [
      markdown({ base: markdownLanguage, extensions: [GFM] }),
      livePreview({ onWikilinkClick: () => {} }, { reveal }),
    ],
  });
  ensureSyntaxTree(state, state.doc.length, 5000);
  const view = new EditorView({ state, parent });
  views.push(view);
  return view;
}

function lineTexts(view: EditorView): string[] {
  return [...view.dom.querySelectorAll('.cm-line')].map((line) => line.textContent ?? '');
}

afterEach(() => {
  for (const view of views.splice(0)) {
    const parent = view.dom.parentElement;
    view.destroy();
    parent?.remove();
  }
});

describe('reading-mode paragraph reflow', () => {
  it('joins a hard-wrapped paragraph into one visual line', () => {
    const view = viewFor('Un lenguaje interpretado tiene\nque decidir qué ejecuta.', false);

    expect(lineTexts(view)).toEqual(['Un lenguaje interpretado tiene que decidir qué ejecuta.']);
  });

  it('leaves the markdown source untouched', () => {
    const doc = 'Un lenguaje interpretado tiene\nque decidir qué ejecuta.';
    const view = viewFor(doc, false);

    expect(view.state.doc.toString()).toBe(doc);
  });

  it('keeps each source line separate in edit mode', () => {
    const view = viewFor('Un lenguaje interpretado tiene\nque decidir qué ejecuta.', true);

    expect(lineTexts(view)).toEqual(['Un lenguaje interpretado tiene', 'que decidir qué ejecuta.']);
  });

  it('does not join across a blank line (separate paragraphs)', () => {
    const view = viewFor('First paragraph.\n\nSecond paragraph.', false);

    expect(lineTexts(view)).toEqual(['First paragraph.', '', 'Second paragraph.']);
  });

  it('keeps a two-space hard break as a break', () => {
    const view = viewFor('Line one.  \nLine two.', false);

    expect(lineTexts(view).length).toBe(2);
  });

  it('joins a hard-wrapped blockquote without leaving its marker visible', () => {
    const view = viewFor('> Guía didáctica, escrita\n> desde cero.', false);

    expect(lineTexts(view)).toEqual(['Guía didáctica, escrita desde cero.']);
  });

  it('joins a hard-wrapped list item without leaving its indentation visible', () => {
    const view = viewFor('- una opción larga\n  que sigue acá', false);

    expect(lineTexts(view)).toEqual(['• una opción larga que sigue acá']);
  });

  it('leaves a fenced code block wrapping exactly as written', () => {
    const view = viewFor('```\nBinary(+)\n  / \\\n```', false);

    expect(lineTexts(view).length).toBe(4);
  });
});

describe('horizontal rule rendering', () => {
  it('draws the rule without also showing its markers', () => {
    const view = viewFor('before\n\n---\n\nafter', false);

    expect(view.dom.querySelector('.cm-atlas-hr')).not.toBeNull();
    expect(view.dom.textContent).not.toContain('---');
  });

  it('reveals the markers on the active line so they stay editable', () => {
    const doc = 'before\n\n---\n\nafter';
    const view = viewFor(doc, true, doc.indexOf('---'));

    expect(view.dom.querySelector('.cm-atlas-hr')).not.toBeNull();
    expect(view.dom.textContent).toContain('---');
  });

  it('keeps the markers hidden on an inactive line while editing', () => {
    const view = viewFor('before\n\n---\n\nafter', true, 0);

    expect(view.dom.textContent).not.toContain('---');
  });

  it('renders an underscore rule the same way', () => {
    const view = viewFor('before\n\n___\n\nafter', false);

    expect(view.dom.querySelector('.cm-atlas-hr')).not.toBeNull();
    expect(view.dom.textContent).not.toContain('___');
  });
});
