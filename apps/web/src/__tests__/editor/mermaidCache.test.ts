import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { ensureSyntaxTree } from '@codemirror/language';
import { languages } from '@codemirror/language-data';
import { EditorSelection, EditorState } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import { GFM } from '@lezer/markdown';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { livePreview, MermaidWidget } from '@/components/editor/livePreviewExtension';

const { render, initialize } = vi.hoisted(() => ({
  render: vi.fn(async (_id: string, code: string) => ({ svg: `<svg data-code="${code}"></svg>` })),
  initialize: vi.fn(),
}));

vi.mock('mermaid', () => ({ default: { initialize, render } }));

const views: EditorView[] = [];

function mermaidDoc(code: string): string {
  return ['intro', '', '```mermaid', code, '```', '', 'after'].join('\n');
}

function viewFor(doc: string): EditorView {
  const parent = document.createElement('div');
  document.body.appendChild(parent);
  const state = EditorState.create({
    doc,
    selection: EditorSelection.cursor(0),
    extensions: [
      markdown({ base: markdownLanguage, extensions: [GFM], codeLanguages: languages }),
      livePreview({ onWikilinkClick: () => {} }, { reveal: false }),
    ],
  });
  ensureSyntaxTree(state, state.doc.length, 5000);
  const view = new EditorView({ state, parent });
  views.push(view);
  return view;
}

async function flushMermaid(): Promise<void> {
  for (let i = 0; i < 5; i += 1) await new Promise((resolve) => setTimeout(resolve, 0));
}

function svgIn(view: EditorView): SVGElement | null {
  return view.dom.querySelector('.cm-atlas-mermaid svg');
}

beforeEach(() => {
  render.mockClear();
  initialize.mockClear();
});

afterEach(() => {
  for (const view of views.splice(0)) {
    const parent = view.dom.parentElement;
    view.destroy();
    parent?.remove();
  }
  delete document.documentElement.dataset.theme;
});

describe('mermaid render cache', () => {
  it('renders a diagram once and paints later widgets with the same source synchronously', async () => {
    const code = 'graph TD; A-->B;';
    const first = viewFor(mermaidDoc(code));

    expect(svgIn(first)).toBeNull();

    await flushMermaid();

    expect(render).toHaveBeenCalledTimes(1);
    expect(svgIn(first)?.getAttribute('data-code')).toBe(code);

    const second = viewFor(mermaidDoc(code));

    expect(svgIn(second)?.getAttribute('data-code')).toBe(code);
    expect(render).toHaveBeenCalledTimes(1);
  });

  it('renders two widgets with the same source from one in-flight render', async () => {
    const code = 'graph LR; X-->Y;';
    const doc = [mermaidDoc(code), '', '```mermaid', code, '```'].join('\n');
    const view = viewFor(doc);

    await flushMermaid();

    expect(view.dom.querySelectorAll('.cm-atlas-mermaid svg')).toHaveLength(2);
    expect(render).toHaveBeenCalledTimes(1);
  });

  it('re-renders live diagrams and drops cached output when the app theme changes', async () => {
    const code = 'graph TD; T-->U;';
    const view = viewFor(mermaidDoc(code));
    await flushMermaid();

    expect(render).toHaveBeenCalledTimes(1);

    document.documentElement.dataset.theme = 'light';
    await flushMermaid();

    expect(render).toHaveBeenCalledTimes(2);
    expect(initialize).toHaveBeenLastCalledWith(expect.objectContaining({ theme: 'default' }));
    expect(svgIn(view)?.getAttribute('data-code')).toBe(code);

    view.destroy();
    views.splice(views.indexOf(view), 1);

    document.documentElement.dataset.theme = 'dark';
    await flushMermaid();

    expect(render).toHaveBeenCalledTimes(2);

    viewFor(mermaidDoc(code));
    await flushMermaid();

    expect(render).toHaveBeenCalledTimes(3);
  });

  it('keeps the widget identity for an unchanged diagram at the same position', () => {
    const widget = new MermaidWidget('graph TD; A-->B;', 10);

    expect(widget.eq(new MermaidWidget('graph TD; A-->B;', 10))).toBe(true);
    expect(widget.eq(new MermaidWidget('graph TD; A-->C;', 10))).toBe(false);
    expect(widget.eq(new MermaidWidget('graph TD; A-->B;', 11))).toBe(false);
  });

  it('reserves the last measured height for a diagram before its SVG lands', async () => {
    const code = 'graph TD; H-->I;';
    const measured = vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (
      this: HTMLElement,
    ) {
      const height = this.classList.contains('cm-atlas-mermaid') ? 120 : 0;
      return { x: 0, y: 0, top: 0, left: 0, bottom: height, right: 0, width: 0, height, toJSON: () => ({}) };
    });

    try {
      expect(new MermaidWidget(code, 0).estimatedHeight).toBe(-1);

      viewFor(mermaidDoc(code));
      await flushMermaid();
      await new Promise((resolve) => requestAnimationFrame(resolve));
      await new Promise((resolve) => requestAnimationFrame(resolve));

      expect(new MermaidWidget(code, 0).estimatedHeight).toBe(120);

      document.documentElement.dataset.theme = 'light';
      const wrap = viewFor(mermaidDoc(code)).dom.querySelector<HTMLElement>('.cm-atlas-mermaid');

      expect(wrap?.querySelector('svg')).toBeNull();
      expect(wrap?.style.minHeight).toBe('120px');

      await flushMermaid();

      expect(wrap?.querySelector('svg')).not.toBeNull();
      expect(wrap?.style.minHeight).toBe('');
    } finally {
      measured.mockRestore();
    }
  });
});
