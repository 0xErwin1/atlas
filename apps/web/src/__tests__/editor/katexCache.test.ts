import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { ensureSyntaxTree } from '@codemirror/language';
import { EditorSelection, EditorState } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import { GFM } from '@lezer/markdown';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { livePreview } from '@/components/editor/livePreviewExtension';
import { flushKatex } from './katexReady';

const renderToString = vi.hoisted(() =>
  vi.fn<(formula: string, options: Record<string, unknown>) => string>(),
);

vi.mock('katex', async (importOriginal) => {
  const actual = await importOriginal<typeof import('katex')>();
  renderToString.mockImplementation((formula, options) => actual.default.renderToString(formula, options));
  return { ...actual, default: { ...actual.default, renderToString } };
});

const views: EditorView[] = [];

function viewFor(doc: string): EditorView {
  const parent = document.createElement('div');
  document.body.appendChild(parent);
  const state = EditorState.create({
    doc,
    selection: EditorSelection.cursor(0),
    extensions: [
      markdown({ base: markdownLanguage, extensions: [GFM] }),
      livePreview({ onWikilinkClick: () => {} }, { reveal: false }),
    ],
  });
  ensureSyntaxTree(state, state.doc.length, 5000);
  const view = new EditorView({ state, parent });
  views.push(view);
  return view;
}

beforeEach(() => {
  renderToString.mockClear();
});

afterEach(() => {
  for (const view of views.splice(0)) {
    const parent = view.dom.parentElement;
    view.destroy();
    parent?.remove();
  }
});

describe('KaTeX render cache', () => {
  it('renders a formula once and serves every later widget with the same formula from the cache', async () => {
    const first = viewFor('The area is $a^2 + b^2$ units');
    await flushKatex();

    expect(first.dom.querySelector('.cm-atlas-math-inline .katex')).not.toBeNull();
    expect(renderToString).toHaveBeenCalledTimes(1);

    const second = viewFor('Again $a^2 + b^2$ here, and $a^2 + b^2$ twice');

    expect(second.dom.querySelectorAll('.cm-atlas-math-inline .katex')).toHaveLength(2);
    expect(renderToString).toHaveBeenCalledTimes(1);
  });

  it('keeps inline and display renders of the same formula apart', async () => {
    viewFor(['$$', 'x_1', '$$', '', 'and $x_1$'].join('\n'));
    await flushKatex();

    expect(renderToString).toHaveBeenCalledTimes(2);
    expect(renderToString.mock.calls.map(([, options]) => options.displayMode)).toEqual(
      expect.arrayContaining([true, false]),
    );
  });

  it('emits HTML only, without the hidden MathML twin', async () => {
    const view = viewFor('Only $\\sqrt{2}$ here');
    await flushKatex();

    expect(renderToString).toHaveBeenLastCalledWith('\\sqrt{2}', expect.objectContaining({ output: 'html' }));
    expect(view.dom.querySelector('.katex-mathml')).toBeNull();
    expect(view.dom.querySelector('.katex-html')).not.toBeNull();
  });
});
