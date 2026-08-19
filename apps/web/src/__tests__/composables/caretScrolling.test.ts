import { afterEach, describe, expect, it, vi } from 'vitest';
import { caretScrollDelta, createScrollableAncestorResolver } from '@/composables/caretScrolling';

/**
 * Keeping the caret in view runs on every keystroke, so the ancestor walk (one
 * `getComputedStyle` per level) must happen once rather than once per character,
 * and a caret already inside the box must cost no scroll write at all.
 */

function scrollable(el: HTMLElement, scrollHeight: number, clientHeight: number): HTMLElement {
  el.style.overflowY = 'auto';
  Object.defineProperty(el, 'scrollHeight', { value: scrollHeight, configurable: true });
  Object.defineProperty(el, 'clientHeight', { value: clientHeight, configurable: true });
  return el;
}

function tree(): { scroller: HTMLElement; leaf: HTMLElement } {
  const scroller = scrollable(document.createElement('div'), 900, 300);
  const middle = document.createElement('div');
  const leaf = document.createElement('div');

  document.body.append(scroller);
  scroller.append(middle);
  middle.append(leaf);

  return { scroller, leaf };
}

afterEach(() => {
  document.body.innerHTML = '';
  vi.restoreAllMocks();
});

describe('caretScrollDelta', () => {
  const box = { top: 100, bottom: 400 };

  it('is zero while the caret sits inside the margin', () => {
    expect(caretScrollDelta({ top: 200, bottom: 220 }, box, 28)).toBe(0);
    expect(caretScrollDelta({ top: 128, bottom: 372 }, box, 28)).toBe(0);
  });

  it('scrolls down by the overshoot below the bottom margin', () => {
    expect(caretScrollDelta({ top: 380, bottom: 400 }, box, 28)).toBe(28);
  });

  it('scrolls up by the overshoot above the top margin', () => {
    expect(caretScrollDelta({ top: 100, bottom: 120 }, box, 28)).toBe(-28);
  });
});

describe('createScrollableAncestorResolver', () => {
  it('finds the nearest ancestor that actually scrolls', () => {
    const { scroller, leaf } = tree();

    expect(createScrollableAncestorResolver()(leaf)).toBe(scroller);
  });

  it('walks the ancestors only once for repeated lookups', () => {
    const { leaf } = tree();
    const resolve = createScrollableAncestorResolver();

    resolve(leaf);
    const styles = vi.spyOn(window, 'getComputedStyle');
    resolve(leaf);
    resolve(leaf);

    expect(styles).not.toHaveBeenCalled();
  });

  it('walks again once the element is re-parented', () => {
    const { leaf } = tree();
    const resolve = createScrollableAncestorResolver();
    resolve(leaf);

    const other = tree();
    other.leaf.append(leaf);

    expect(resolve(leaf)).toBe(other.scroller);
  });

  it('keeps looking while nothing scrolls yet', () => {
    const plain = document.createElement('div');
    const leaf = document.createElement('div');
    document.body.append(plain);
    plain.append(leaf);

    const resolve = createScrollableAncestorResolver();
    expect(resolve(leaf)).toBeNull();

    scrollable(plain, 900, 300);
    expect(resolve(leaf)).toBe(plain);
  });
});
