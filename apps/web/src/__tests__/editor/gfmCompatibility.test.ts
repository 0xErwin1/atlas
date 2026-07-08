import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { ensureSyntaxTree } from '@codemirror/language';
import { languages } from '@codemirror/language-data';
import { EditorSelection, EditorState } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import { GFM } from '@lezer/markdown';
import { afterEach, describe, expect, it } from 'vitest';
import { livePreview } from '@/components/editor/livePreviewExtension';

const views: EditorView[] = [];
const NOTE_ID = '019ed5fa-6df7-7201-97ce-a99abae541c1';

function viewFor(doc: string, cursor = 0, reveal = false): EditorView {
  const parent = document.createElement('div');
  document.body.appendChild(parent);
  const state = EditorState.create({
    doc,
    selection: EditorSelection.cursor(cursor),
    extensions: [
      markdown({ base: markdownLanguage, extensions: [GFM], codeLanguages: languages }),
      livePreview({ onWikilinkClick: () => {} }, { reveal, titles: { [NOTE_ID]: 'Resolved Note' } }),
    ],
  });
  ensureSyntaxTree(state, state.doc.length, 5000);
  const view = new EditorView({ state, parent });
  views.push(view);
  return view;
}

function text(view: EditorView): string {
  return view.dom.textContent ?? '';
}

afterEach(() => {
  for (const view of views.splice(0)) {
    const parent = view.dom.parentElement;
    view.destroy();
    parent?.remove();
  }
});

const gfmCompatibilityMatrix = [
  { surface: 'tables', expectation: 'rendered block widget with inline cell formatting' },
  { surface: 'task lists', expectation: 'rendered checkbox controls replace GFM task markers' },
  { surface: 'emphasis/strong/strikethrough', expectation: 'markers hidden while content remains visible' },
  { surface: 'links/autolinks', expectation: 'links render as link text without raw URL/angle syntax' },
  { surface: 'headings', expectation: 'ATX marker hidden while heading text remains visible' },
  {
    surface: 'lists',
    expectation: 'unordered markers render as bullets and ordered markers remain readable',
  },
  { surface: 'images', expectation: 'image markdown renders as an image with alt/src' },
  { surface: 'Mermaid', expectation: 'mermaid fences render as a block widget without raw fence markers' },
  { surface: 'wikilinks', expectation: 'wikilinks render as resolved titles' },
  {
    surface: 'math coexistence',
    expectation: 'math renders alongside GFM, including inside rendered table cells',
  },
] as const;

describe('live preview GFM compatibility matrix', () => {
  it('documents the audited surfaces for the live-preview editor', () => {
    expect(gfmCompatibilityMatrix.map((entry) => entry.surface)).toEqual([
      'tables',
      'task lists',
      'emphasis/strong/strikethrough',
      'links/autolinks',
      'headings',
      'lists',
      'images',
      'Mermaid',
      'wikilinks',
      'math coexistence',
    ]);
    expect(gfmCompatibilityMatrix.every((entry) => entry.expectation.length > 0)).toBe(true);
  });

  it('renders tables with inline GFM, wikilinks, links, and math without exposing raw cell markup', () => {
    const doc = [
      '| Feature | Preview |',
      '| --- | --- |',
      '| Math | $a^2$ |',
      '| Format | **bold** and ~~gone~~ |',
      '| Link | [Atlas](https://atlas.local) and [[Table Note]] |',
    ].join('\n');

    const view = viewFor(doc);
    const table = view.dom.querySelector('table.cm-atlas-table');

    expect(table?.querySelectorAll('thead th')).toHaveLength(2);
    expect(table?.querySelectorAll('tbody tr')).toHaveLength(3);
    expect(table?.querySelector('.cm-atlas-math-inline .katex')?.textContent).toContain('a');
    expect(table?.querySelector('.cm-atlas-strong')?.textContent).toBe('bold');
    expect(table?.querySelector('.cm-atlas-strike')?.textContent).toBe('gone');
    expect(table?.querySelector('a.cm-atlas-link')?.getAttribute('href')).toBe('https://atlas.local');
    expect(table?.querySelector('.cm-atlas-wikilink')?.textContent).toBe('Table Note');
    expect(text(view)).not.toContain('$a^2$');
    expect(text(view)).not.toContain('**bold**');
    expect(text(view)).not.toContain('~~gone~~');
    expect(text(view)).not.toContain('[[Table Note]]');
  });

  it('renders task lists, headings, unordered lists, emphasis, strong, strikethrough, and inline links', () => {
    const doc = [
      '# Heading',
      '',
      '- [x] Done',
      '- [ ] Todo',
      '- bullet',
      '1. ordered',
      '',
      'This has *em*, **strong**, ~~strike~~, and [link](https://atlas.local/docs).',
    ].join('\n');

    const view = viewFor(doc);
    const boxes = [...view.dom.querySelectorAll<HTMLInputElement>('input.cm-atlas-checkbox')];

    expect(text(view)).toContain('Heading');
    expect(text(view)).not.toContain('# Heading');
    expect(boxes).toHaveLength(2);
    expect(boxes.map((box) => box.checked)).toEqual([true, false]);
    expect(view.dom.querySelector('.cm-atlas-bullet')?.textContent).toBe('•');
    expect(text(view)).toContain('1. ordered');
    expect(view.dom.querySelector('.cm-atlas-em')?.textContent).toBe('em');
    expect(view.dom.querySelector('.cm-atlas-strong')?.textContent).toBe('strong');
    expect(view.dom.querySelector('.cm-atlas-strike')?.textContent).toBe('strike');
    expect(view.dom.querySelector('a.cm-atlas-link')?.textContent).toBe('link');
    expect(text(view)).not.toContain('[link](https://atlas.local/docs)');
  });

  it('renders inline markdown inside standard link labels', () => {
    const doc = '[**bold**](https://atlas.local/bold) and [$x$](https://atlas.local/math)';

    const view = viewFor(doc);
    const links = [...view.dom.querySelectorAll<HTMLAnchorElement>('a.cm-atlas-link')];

    expect(links).toHaveLength(2);
    expect(links[0]?.querySelector('.cm-atlas-strong')?.textContent).toBe('bold');
    expect(links[0]?.textContent).toBe('bold');
    expect(links[1]?.querySelector('.cm-atlas-math-inline .katex')?.textContent).toContain('x');
    expect(text(view)).not.toContain('**bold**');
    expect(text(view)).not.toContain('$x$');
  });

  it('renders autolinks, images, Mermaid fences, wikilinks, and adjacent math without corrupting source syntax', () => {
    const doc = [
      '<https://atlas.local/autolink>',
      '',
      '![Diagram](https://atlas.local/diagram.png)',
      '',
      '```mermaid',
      'graph TD; A-->B;',
      '```',
      '',
      `See [[${NOTE_ID}|Old Note]] and inline $x + y$.`,
    ].join('\n');

    const view = viewFor(doc);
    const autolink = view.dom.querySelector<HTMLAnchorElement>(
      'a.cm-atlas-link[href="https://atlas.local/autolink"]',
    );
    const image = view.dom.querySelector<HTMLImageElement>('img.cm-atlas-img');

    expect(autolink?.textContent).toBe('https://atlas.local/autolink');
    expect(text(view)).not.toContain('<https://atlas.local/autolink>');
    expect(image?.getAttribute('alt')).toBe('Diagram');
    expect(image?.getAttribute('src')).toBe('https://atlas.local/diagram.png');
    expect(view.dom.querySelector('.cm-atlas-mermaid')).not.toBeNull();
    expect(text(view)).not.toContain('```mermaid');
    expect(view.dom.querySelector('.cm-atlas-wikilink')?.textContent).toBe('Resolved Note');
    expect(view.dom.querySelector('.cm-atlas-math-inline .katex')?.textContent).toContain('x');
    expect(text(view)).not.toContain(`[[${NOTE_ID}|Old Note]]`);
    expect(text(view)).not.toContain('$x + y$');
  });
});
