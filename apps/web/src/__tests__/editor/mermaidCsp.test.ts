import { afterEach, describe, expect, it } from 'vitest';
import { attachMermaidSvg } from '@/components/editor/livePreviewExtension';

const NONCE = 'test-nonce-abc123';

afterEach(() => {
  document.head.replaceChildren();
  document.body.replaceChildren();
});

describe('attachMermaidSvg', () => {
  it('adds the document style nonce to every style in Mermaid SVG output', () => {
    const documentStyle = document.createElement('style');
    documentStyle.nonce = NONCE;
    document.head.appendChild(documentStyle);

    const container = document.createElement('div');
    document.body.appendChild(container);

    attachMermaidSvg(
      container,
      '<svg><style>.node { fill: red; }</style><g><style>.edge { stroke: blue; }</style></g></svg>',
    );

    const styles = container.querySelectorAll('style');
    expect(styles).toHaveLength(2);
    for (const style of styles) expect(style.getAttribute('nonce')).toBe(NONCE);
  });

  it('attaches Mermaid SVG output unchanged when the document has no style nonce', () => {
    const container = document.createElement('div');

    attachMermaidSvg(container, '<svg><style>.node { fill: red; }</style><g class="node" /></svg>');

    expect(container.querySelector('style')?.hasAttribute('nonce')).toBe(false);
    expect(container.querySelector('.node')).not.toBeNull();
  });
});
