import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { ensureSyntaxTree } from '@codemirror/language';
import { languages } from '@codemirror/language-data';
import { EditorSelection, EditorState } from '@codemirror/state';
import type { EditorView } from '@codemirror/view';
import { GFM } from '@lezer/markdown';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { buildDecorations } from '@/components/editor/livePreviewExtension';

interface EnteredNode {
  name: string;
  from: number;
  to: number;
}

const entered = vi.hoisted<EnteredNode[]>(() => []);

/**
 * Wraps the tree `ensureSyntaxTree` hands to the decorator so every node the
 * walk enters is recorded, without changing what the walk sees.
 */
vi.mock('@codemirror/language', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@codemirror/language')>();
  type Tree = NonNullable<ReturnType<typeof actual.ensureSyntaxTree>>;
  type IterateSpec = Parameters<Tree['iterate']>[0];

  const spied = (tree: Tree): Tree => {
    const wrapped: Tree = Object.assign(Object.create(Object.getPrototypeOf(tree)), tree);
    wrapped.iterate = (spec: IterateSpec) =>
      tree.iterate({
        ...spec,
        enter: (node) => {
          entered.push({ name: node.name, from: node.from, to: node.to });
          return spec.enter(node);
        },
      });
    return wrapped;
  };

  return {
    ...actual,
    ensureSyntaxTree: (...args: Parameters<typeof actual.ensureSyntaxTree>) => {
      const tree = actual.ensureSyntaxTree(...args);
      return tree === null ? null : spied(tree);
    },
  };
});

const HTML_BLOCK = '<div class="callout">\n  <p>boxed</p>\n</div>';

function viewWithViewport(state: EditorState, viewportTo: number): EditorView {
  return {
    state,
    viewport: { from: 0, to: viewportTo },
    visibleRanges: [{ from: 0, to: viewportTo }],
  } as unknown as EditorView;
}

beforeEach(() => {
  entered.splice(0);
});

describe('viewport-scoped decoration walk', () => {
  it('never enters an HTML block that lies entirely outside the visible ranges', () => {
    const head = `${HTML_BLOCK}\n\nA visible paragraph with **bold** text.\n\n`;
    const filler = Array.from({ length: 40 }, (_, i) => `Paragraph ${i} past the viewport.`).join('\n\n');
    const doc = `${head}${filler}\n\n${HTML_BLOCK}\n`;
    const state = EditorState.create({
      doc,
      selection: EditorSelection.cursor(0),
      extensions: [markdown({ base: markdownLanguage, extensions: [GFM], codeLanguages: languages })],
    });
    ensureSyntaxTree(state, doc.length, 5000);
    entered.splice(0);

    const viewportTo = head.length;
    buildDecorations(viewWithViewport(state, viewportTo), { onWikilinkClick: () => {} }, true, {});

    const htmlBlocks = entered.filter((node) => node.name === 'HTMLBlock');
    expect(htmlBlocks.length).toBeGreaterThan(0);
    expect(htmlBlocks.every((node) => node.from < viewportTo)).toBe(true);

    // Lezer enters a node that merely touches the window edge; nothing beyond it.
    expect(entered.every((node) => node.from <= viewportTo)).toBe(true);
  });
});
