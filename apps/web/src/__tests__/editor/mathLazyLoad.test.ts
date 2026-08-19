import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { ensureSyntaxTree } from '@codemirror/language';
import { EditorSelection, EditorState } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import { GFM } from '@lezer/markdown';
import { afterEach, describe, expect, it } from 'vitest';
import { livePreview } from '@/components/editor/livePreviewExtension';
import { flushKatex } from './katexReady';

// The extension caches the KaTeX module for the lifetime of the module registry,
// so these assertions need a file of their own: any earlier math render in the
// same file would have already resolved the lazy import.

const views: EditorView[] = [];

function viewFor(doc: string, cursor = 0, reveal = false): EditorView {
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

afterEach(() => {
  for (const view of views.splice(0)) {
    const parent = view.dom.parentElement;
    view.destroy();
    parent?.remove();
  }
});

describe('live preview lazy KaTeX loading', () => {
  it('paints an empty widget until the lazy import resolves, never raw TeX', async () => {
    const view = viewFor('The area is $a^2$ units');
    const widget = view.dom.querySelector('.cm-atlas-math-inline');

    expect(widget).not.toBeNull();
    expect(widget?.querySelector('.katex')).toBeNull();
    expect(widget?.textContent).toBe('');
    expect(view.dom.textContent).not.toContain('a^2');

    await flushKatex();

    expect(widget?.querySelector('.katex')).not.toBeNull();
    expect(widget?.textContent).toContain('a');
    expect(widget?.classList.contains('cm-atlas-math-error')).toBe(false);
  });

  it('renders block math synchronously once KaTeX has loaded', async () => {
    const first = viewFor(['$$', '\\int_0^1 x dx', '$$'].join('\n'));
    await flushKatex();

    expect(first.dom.querySelector('.cm-atlas-math-block .katex')).not.toBeNull();

    const second = viewFor(['$$', '\\sum_{i=1}^n i', '$$'].join('\n'));

    expect(second.dom.querySelector('.cm-atlas-math-block .katex')).not.toBeNull();
  });

  it('falls back to the accessible error surface for invalid math after loading', async () => {
    const view = viewFor('Broken $\\frac{$ math');
    await flushKatex();

    const fallback = view.dom.querySelector('.cm-atlas-math-inline.cm-atlas-math-error');

    expect(fallback).not.toBeNull();
    expect(fallback?.getAttribute('role')).toBe('note');
    expect(fallback?.textContent).toContain('Invalid math');
  });
});
