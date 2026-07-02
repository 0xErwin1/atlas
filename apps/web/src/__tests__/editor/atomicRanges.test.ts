import { Decoration } from '@codemirror/view';
import { describe, expect, it } from 'vitest';
import { isReplaceDeco } from '@/components/editor/livePreviewExtension';

class StubWidget {
  toDOM(): HTMLElement {
    return document.createElement('span');
  }
}

/**
 * `isReplaceDeco` is the discriminator that keeps the live-preview `atomicRanges`
 * set limited to replaced/hidden ranges. If a visible mark (inline code,
 * emphasis, link) ever leaked into that set it would become atomic — the caret
 * could not enter it and a single backspace would delete the whole span — which
 * was the inline-code editing bug this guards against.
 */
describe('isReplaceDeco', () => {
  it('excludes styling mark decorations so they stay editable', () => {
    expect(isReplaceDeco(Decoration.mark({ class: 'cm-atlas-code' }))).toBe(false);
    expect(isReplaceDeco(Decoration.mark({ class: 'cm-atlas-em' }))).toBe(false);
    expect(isReplaceDeco(Decoration.mark({ class: 'cm-atlas-link' }))).toBe(false);
  });

  it('excludes line decorations', () => {
    expect(isReplaceDeco(Decoration.line({ class: 'cm-atlas-fenced' }))).toBe(false);
  });

  it('includes hidden-mark and widget replace decorations', () => {
    expect(isReplaceDeco(Decoration.replace({}))).toBe(true);
    expect(isReplaceDeco(Decoration.replace({ widget: new StubWidget() }))).toBe(true);
  });
});
