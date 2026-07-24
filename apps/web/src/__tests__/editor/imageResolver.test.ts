import type { EditorView } from '@codemirror/view';
import { describe, expect, it, vi } from 'vitest';
import { ImageWidget } from '@/components/editor/livePreviewExtension';

/**
 * A rendered Markdown image whose source is an Atlas API path cannot be loaded by
 * the webview directly (see `useApiImageSrc`). The widget therefore accepts an
 * optional resolver and defers its `src` to whatever that returns.
 */

const view = { requestMeasure: () => {} } as unknown as EditorView;

async function settle(): Promise<void> {
  await Promise.resolve();
  await Promise.resolve();
}

describe('ImageWidget source resolution', () => {
  it('sets the source directly when the host supplies no resolver', () => {
    const img = new ImageWidget('/api/a/content', 'diagram').toDOM(view) as HTMLImageElement;

    expect(img.tagName).toBe('IMG');
    expect(img.getAttribute('src')).toBe('/api/a/content');
  });

  it('renders the resolved source so API-hosted images load through the platform', async () => {
    const resolve = vi.fn().mockResolvedValue('blob:1');

    const img = new ImageWidget('/api/a/content', 'diagram', resolve).toDOM(view) as HTMLImageElement;
    await settle();

    expect(resolve).toHaveBeenCalledWith('/api/a/content');
    expect(img.getAttribute('src')).toBe('blob:1');
    expect(img.alt).toBe('diagram');
  });

  it('leaves an unresolvable image without a source so its alt text stands in', async () => {
    const img = new ImageWidget('/api/gone', 'diagram', vi.fn().mockResolvedValue(null)).toDOM(
      view,
    ) as HTMLImageElement;
    await settle();

    expect(img.hasAttribute('src')).toBe(false);
    expect(img.alt).toBe('diagram');
  });

  it('never resolves a disallowed scheme, collapsing it to alt text as before', () => {
    const resolve = vi.fn();

    const el = new ImageWidget('javascript:alert(1)', 'diagram', resolve).toDOM(view);

    expect(el.tagName).toBe('SPAN');
    expect(el.textContent).toBe('diagram');
    expect(resolve).not.toHaveBeenCalled();
  });

  it('treats widgets as equal only when they agree on the resolver', () => {
    const resolve = vi.fn();

    expect(new ImageWidget('/a', 'x', resolve).eq(new ImageWidget('/a', 'x', resolve))).toBe(true);
    expect(new ImageWidget('/a', 'x', resolve).eq(new ImageWidget('/a', 'x'))).toBe(false);
  });
});
