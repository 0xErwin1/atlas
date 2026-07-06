import { describe, expect, it } from 'vitest';
import { ImageWidget, inlineNode } from '@/components/editor/livePreviewExtension';
import type { InlineToken } from '@/lib/livePreview';

/**
 * Guards the stored DOM XSS fix (ATL-83): a live-preview link or image whose URL
 * carries a dangerous scheme (`javascript:`, `data:`, ...) must never reach the
 * DOM as a clickable anchor or an image `src`. Safe URLs keep working.
 */

const ctx = { titles: {}, onWikilinkClick: () => {} };

function linkNode(url: string): Node {
  const token: InlineToken = { type: 'link', value: 'x', url };
  return inlineNode(token, ctx);
}

describe('inlineNode link safety (ATL-83)', () => {
  it('renders a javascript: link as plain text, not an anchor', () => {
    const node = linkNode('javascript:alert(1)');
    expect(node).toBeInstanceOf(Text);
    expect((node as HTMLElement).tagName).toBeUndefined();
    expect(node.textContent).toBe('x');
  });

  it('neutralizes a control-character obfuscated javascript: link', () => {
    const node = linkNode('java\tscript:alert(1)');
    expect(node).toBeInstanceOf(Text);
    expect(node.textContent).toBe('x');
  });

  it('renders a normal https link as a working anchor', () => {
    const node = linkNode('https://example.com');
    const a = node as HTMLAnchorElement;
    expect(a.tagName).toBe('A');
    expect(a.getAttribute('href')).toBe('https://example.com');
    expect(a.rel).toBe('noopener noreferrer');
  });

  it('renders a relative link as a working anchor', () => {
    const a = linkNode('/foo') as HTMLAnchorElement;
    expect(a.tagName).toBe('A');
    expect(a.getAttribute('href')).toBe('/foo');
  });

  it('renders an anchor-only link as a working anchor', () => {
    const a = linkNode('#anchor') as HTMLAnchorElement;
    expect(a.tagName).toBe('A');
    expect(a.getAttribute('href')).toBe('#anchor');
  });
});

describe('ImageWidget src safety (ATL-83)', () => {
  it('does not set a javascript: src and collapses to alt text', () => {
    const el = new ImageWidget('javascript:alert(1)', 'alt').toDOM();
    expect(el.tagName).toBe('SPAN');
    expect(el.querySelector('img')).toBeNull();
    expect(el.textContent).toBe('alt');
  });

  it('does not set a data: src and collapses to alt text', () => {
    const el = new ImageWidget('data:text/html,<script>alert(1)</script>', 'alt').toDOM();
    expect(el.tagName).toBe('SPAN');
    expect(el.querySelector('img')).toBeNull();
  });

  it('renders an http(s) image as a real <img> with the src set', () => {
    const el = new ImageWidget('https://example.com/x.png', 'alt').toDOM() as HTMLImageElement;
    expect(el.tagName).toBe('IMG');
    expect(el.getAttribute('src')).toBe('https://example.com/x.png');
    expect(el.alt).toBe('alt');
  });
});
