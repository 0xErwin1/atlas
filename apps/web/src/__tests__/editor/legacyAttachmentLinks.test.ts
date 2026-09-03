import { markdown, markdownLanguage } from '@codemirror/lang-markdown';
import { ensureSyntaxTree } from '@codemirror/language';
import { EditorSelection, EditorState } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import { GFM } from '@lezer/markdown';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { livePreview } from '@/components/editor/livePreviewExtension';

/**
 * A stored body written before the V2 cutover embeds attachment links in the
 * retired `/api/workspaces/...` form. The server keeps no alias for it, so the
 * renderer must emit the V2 form for every `href` and `src` it produces, while
 * leaving the stored markdown untouched.
 */

const LEGACY_IMAGE = '/api/workspaces/acme/tasks/ATL-1/attachments/img-1/content';
const LEGACY_FILE = '/api/workspaces/acme/tasks/ATL-1/attachments/file-1/content';
const CURRENT_IMAGE = '/api/v2/acta/workspaces/acme/tasks/ATL-1/attachments/img-1/content';
const CURRENT_FILE = '/api/v2/acta/workspaces/acme/tasks/ATL-1/attachments/file-1/content';

const views: EditorView[] = [];

function viewFor(doc: string, resolveImageSrc?: (url: string) => Promise<string | null>): EditorView {
  const parent = document.createElement('div');
  document.body.appendChild(parent);

  const state = EditorState.create({
    doc,
    selection: EditorSelection.cursor(doc.length),
    extensions: [
      markdown({ base: markdownLanguage, extensions: [GFM] }),
      livePreview({ onWikilinkClick: () => {}, resolveImageSrc }, { reveal: false, titles: {} }),
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

describe('legacy attachment links in stored content', () => {
  const doc = [
    `![Diagram](${LEGACY_IMAGE})`,
    '',
    `[Spec](${LEGACY_FILE})`,
    '',
    '<div>',
    `  <a href="${LEGACY_FILE}">Raw</a>`,
    '</div>',
    '',
    'after',
  ].join('\n');

  it('renders V2 src and href for a V1 image and V1 links without changing the document', () => {
    const view = viewFor(doc);

    const image = view.dom.querySelector<HTMLImageElement>('img.cm-atlas-img');
    const links = [...view.dom.querySelectorAll<HTMLAnchorElement>('a[href]')].map((a) =>
      a.getAttribute('href'),
    );

    expect(image?.getAttribute('src')).toBe(CURRENT_IMAGE);
    expect(links).toEqual([CURRENT_FILE, CURRENT_FILE]);
    expect(view.state.doc.toString()).toBe(doc);
  });

  it('hands the image resolver the V2 path', () => {
    const resolveImageSrc = vi.fn().mockResolvedValue('blob:1');

    viewFor(doc, resolveImageSrc);

    expect(resolveImageSrc).toHaveBeenCalledWith(CURRENT_IMAGE);
  });

  it('renders V2 links unchanged', () => {
    const view = viewFor(`![Diagram](${CURRENT_IMAGE})\n\n[Spec](${CURRENT_FILE})\n`);

    expect(view.dom.querySelector('img.cm-atlas-img')?.getAttribute('src')).toBe(CURRENT_IMAGE);
    expect(view.dom.querySelector('a[href]')?.getAttribute('href')).toBe(CURRENT_FILE);
  });
});
