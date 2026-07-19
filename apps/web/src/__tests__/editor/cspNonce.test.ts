import { EditorState } from '@codemirror/state';
import { EditorView } from '@codemirror/view';
import { afterEach, describe, expect, it } from 'vitest';
import { cspNonceExtension, documentStyleNonce } from '@/components/editor/cspNonce';

const NONCE = 'test-nonce-abc123';

const views: EditorView[] = [];

/**
 * Replicates what the Tauri asset pipeline does to `index.html`: an inline
 * `<style>` carrying the CSP nonce, present before the editor mounts.
 */
function stampNoncedStyle(): void {
  const style = document.createElement('style');
  style.setAttribute('nonce', NONCE);
  document.head.appendChild(style);
}

afterEach(() => {
  for (const view of views) view.destroy();
  views.length = 0;

  for (const style of document.querySelectorAll('style')) style.remove();
});

describe('documentStyleNonce', () => {
  it('returns the empty string when no nonced style exists', () => {
    expect(documentStyleNonce()).toBe('');
  });

  it('returns the nonce stamped on the index.html inline style', () => {
    stampNoncedStyle();
    expect(documentStyleNonce()).toBe(NONCE);
  });
});

describe('cspNonceExtension', () => {
  it('resolves to no extension when the document has no nonce', () => {
    const state = EditorState.create({ extensions: [cspNonceExtension()] });
    expect(state.facet(EditorView.cspNonce)).toBe('');
  });

  it('provides the stamped nonce through the cspNonce facet', () => {
    stampNoncedStyle();
    const state = EditorState.create({ extensions: [cspNonceExtension()] });
    expect(state.facet(EditorView.cspNonce)).toBe(NONCE);
  });

  it('makes the editor-injected stylesheet carry the nonce', () => {
    stampNoncedStyle();

    const parent = document.createElement('div');
    document.body.appendChild(parent);
    const view = new EditorView({
      state: EditorState.create({ doc: 'hello', extensions: [cspNonceExtension()] }),
      parent,
    });
    views.push(view);

    const injected = [...document.head.querySelectorAll('style')].filter((style) =>
      (style.textContent ?? '').includes('.cm-'),
    );
    expect(injected.length).toBeGreaterThan(0);
    for (const style of injected) {
      expect(style.getAttribute('nonce')).toBe(NONCE);
    }
  });
});
