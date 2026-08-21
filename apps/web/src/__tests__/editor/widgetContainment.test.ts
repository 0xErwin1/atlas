import { describe, expect, it } from 'vitest';
import { atlasMarkdownThemeSpec } from '@/components/editor/theme';

/**
 * Block widgets (diagrams, tables, display math) repaint independently of the
 * prose around them, so a scroll over a long note does not re-rasterise every
 * sibling when one widget's subtree changes. Horizontally scrolling wrappers keep
 * their own scrollbar: `content` containment clips descendants to the padding
 * box, never the element's own scrollbar.
 */
describe('widget containment', () => {
  it.each([
    '.cm-atlas-mermaid',
    '.cm-atlas-table-wrap',
    '.cm-atlas-math-block',
  ])('isolates %s layout and paint from its siblings', (selector) => {
    expect(atlasMarkdownThemeSpec[selector]?.contain).toBe('content');
  });
});
