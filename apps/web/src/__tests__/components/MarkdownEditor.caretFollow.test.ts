import { EditorView } from '@codemirror/view';
import { mount } from '@vue/test-utils';
import { createPinia } from 'pinia';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import MarkdownEditor from '@/components/editor/MarkdownEditor.vue';

/**
 * Following the caret costs a caret measurement, an ancestor walk and possibly a
 * scroll write. Typing fires it far more often than the screen repaints, so the
 * work has to collapse to one frame no matter how many characters land in it.
 */

// jsdom has no layout, so CodeMirror's caret measurement needs a Range that at
// least answers the geometry calls; an empty rect list makes `coordsAtPos` report
// "unmeasurable", which is the honest answer here.
beforeAll(() => {
  const empty = Object.assign([], { item: () => null }) as unknown as DOMRectList;
  Range.prototype.getClientRects = () => empty;
  Range.prototype.getBoundingClientRect = () => new DOMRect();
});

let frames: FrameRequestCallback[] = [];

function runFrames(): void {
  const pending = frames;
  frames = [];
  for (const frame of pending) frame(0);
}

function editor(body = '', props: Record<string, unknown> = {}) {
  const wrapper = mount(MarkdownEditor, {
    props: { body, embeddedControls: false, editable: true, ...props },
    global: { plugins: [createPinia()] },
    attachTo: document.body,
  });

  const view = EditorView.findFromDOM(wrapper.element as HTMLElement);
  if (view === null) throw new Error('no editor view');

  return { wrapper, view };
}

function type(view: EditorView, text: string): void {
  const at = view.state.selection.main.head;
  view.dispatch({
    changes: { from: at, insert: text },
    selection: { anchor: at + text.length },
    userEvent: 'input.type',
  });
}

describe('MarkdownEditor caret following', () => {
  beforeEach(() => {
    frames = [];
    vi.stubGlobal('requestAnimationFrame', (cb: FrameRequestCallback) => frames.push(cb));
    vi.stubGlobal('cancelAnimationFrame', () => {
      frames = [];
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
    document.body.innerHTML = '';
  });

  it('measures the caret once for a burst of keystrokes', () => {
    const { view } = editor('Notes');
    const measures = vi.spyOn(EditorView.prototype, 'coordsAtPos');
    frames = [];

    type(view, 'a');
    type(view, 'b');
    type(view, 'c');

    expect(measures).not.toHaveBeenCalled();

    runFrames();

    expect(measures).toHaveBeenCalledTimes(1);
  });

  it('measures again for the keystrokes that follow the frame', () => {
    const { view } = editor('Notes');
    const measures = vi.spyOn(EditorView.prototype, 'coordsAtPos');

    type(view, 'a');
    runFrames();
    type(view, 'b');
    type(view, 'c');
    runFrames();

    expect(measures).toHaveBeenCalledTimes(2);
  });

  it('never measures when the host opts out of caret following', () => {
    const { view } = editor('Notes', { followCaret: false });
    const measures = vi.spyOn(EditorView.prototype, 'coordsAtPos');

    type(view, 'a');
    runFrames();

    expect(measures).not.toHaveBeenCalled();
  });
});

describe('MarkdownEditor wikilink trigger', () => {
  it('reports the query typed after the trigger', () => {
    const { wrapper, view } = editor('');

    type(view, '[[Design');

    expect(wrapper.emitted('wikilink-query')?.at(-1)?.[0]).toBe('Design');
  });

  it('reports no query inside a code fence', () => {
    const { wrapper, view } = editor('```\n\n```');
    view.dispatch({ selection: { anchor: 4 } });

    type(view, '[[Design');

    expect(wrapper.emitted('wikilink-query')?.at(-1)?.[0]).toBeNull();
  });

  it('reports no query inside inline code', () => {
    const { wrapper, view } = editor('`code`');
    view.dispatch({ selection: { anchor: 5 } });

    type(view, '[[Design');

    expect(wrapper.emitted('wikilink-query')?.at(-1)?.[0]).toBeNull();
  });

  it('reports no query once the trigger is closed', () => {
    const { wrapper, view } = editor('');

    type(view, '[[Design]]');

    expect(wrapper.emitted('wikilink-query')?.at(-1)?.[0]).toBeNull();
  });
});
