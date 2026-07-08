import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { ensureSyntaxTree } from '@codemirror/language';
import { EditorSelection, EditorState } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import { GFM } from '@lezer/markdown';
import { afterEach, describe, expect, it } from 'vitest';
import { livePreview } from '@/components/editor/livePreviewExtension';

const views: EditorView[] = [];

function viewFor(doc: string, cursor = doc.length, reveal = true): EditorView {
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

describe('live preview math rendering', () => {
  it('renders inactive inline math and reveals its source on the active line', () => {
    const inactive = viewFor('The area is $a^2$ units', 0, false);

    expect(inactive.dom.querySelector('.cm-atlas-math-inline .katex')).not.toBeNull();
    expect(inactive.dom.textContent).not.toContain('$a^2$');

    const active = viewFor('The area is $a^2$ units', 'The area is $a'.length, true);

    expect(active.dom.querySelector('.cm-atlas-math-inline')).toBeNull();
    expect(active.dom.textContent).toContain('$a^2$');
  });

  it('renders inactive block math as a block widget and reveals it when active', () => {
    const doc = ['intro', '', '$$', '\\int_0^1 x dx', '$$', '', 'after'].join('\n');
    const inactive = viewFor(doc, 0, false);

    expect(inactive.dom.querySelector('.cm-atlas-math-block .katex')).not.toBeNull();
    expect(inactive.dom.textContent).not.toContain('$$');

    const active = viewFor(doc, doc.indexOf('x dx'), true);

    expect(active.dom.querySelector('.cm-atlas-math-block')).toBeNull();
    expect(active.dom.textContent).toContain('$$');
    expect(active.dom.textContent).toContain('\\int_0^1 x dx');
  });

  it('shows accessible fallbacks for invalid inline and block math without breaking the editor', () => {
    const inline = viewFor('Broken $\\frac{$ math', 0, false);
    const inlineFallback = inline.dom.querySelector('.cm-atlas-math-error');

    expect(inlineFallback).not.toBeNull();
    expect(inlineFallback?.getAttribute('role')).toBe('note');
    expect(inlineFallback?.textContent).toContain('Invalid math');
    expect(inline.dom.textContent).toContain('Broken');
    expect(inline.dom.textContent).toContain('math');

    const block = viewFor(['Before', '', '$$', '\\frac{', '$$', '', 'After'].join('\n'), 0, false);
    const blockFallback = block.dom.querySelector('.cm-atlas-math-block.cm-atlas-math-error');

    expect(blockFallback).not.toBeNull();
    expect(blockFallback?.getAttribute('role')).toBe('note');
    expect(blockFallback?.textContent).toContain('Invalid math');
    expect(block.dom.textContent).toContain('Before');
    expect(block.dom.textContent).toContain('After');
  });

  it('does not render math inside code spans or fenced blocks', () => {
    const inlineCode = viewFor('Use `$x$` literally', 0, false);

    expect(inlineCode.dom.querySelector('.cm-atlas-math-inline')).toBeNull();
    expect(inlineCode.dom.textContent).toContain('$x$');

    const repeatedBackticks = viewFor('Use ``$x$`` literally', 0, false);

    expect(repeatedBackticks.dom.querySelector('.cm-atlas-math-inline')).toBeNull();
    expect(repeatedBackticks.dom.textContent).toContain('$x$');

    const fenced = viewFor(['```', '$x$', '$$x$$', '```'].join('\n'), 0, false);

    expect(fenced.dom.querySelector('.cm-atlas-math-inline')).toBeNull();
    expect(fenced.dom.querySelector('.cm-atlas-math-block')).toBeNull();
    expect(fenced.dom.textContent).toContain('$x$');
  });
});
