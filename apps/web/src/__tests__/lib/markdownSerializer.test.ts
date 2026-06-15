import { describe, expect, it } from 'vitest';
import { docToMarkdown, markdownToDoc } from '../../lib/markdownSerializer';

/**
 * Gating test for the Tiptap + prosemirror-markdown serializer spike (REQ-W15, T17).
 *
 * PASS criterion: md → doc → md is byte-stable for each case, modulo the
 * pre-agreed canonicalization rules listed below. Tests MUST be green before
 * any editor UI work (T19-T20).
 *
 * Canonicalization applied to BOTH the input and the round-tripped output so
 * comparisons remain fair:
 * - Trailing whitespace per line stripped
 * - Multiple consecutive blank lines collapsed to one
 * - Leading/trailing blank lines stripped
 *
 * Frontmatter is stripped BEFORE passing to the editor (lib/frontmatter); the
 * serializer corpus here contains no frontmatter.
 */
function canonical(md: string): string {
  return md
    .split('\n')
    .map((l) => l.trimEnd())
    .join('\n')
    .replace(/\n{3,}/g, '\n\n')
    .replace(/^\n+|\n+$/g, '');
}

function roundTrip(md: string): string {
  const doc = markdownToDoc(md);
  return docToMarkdown(doc);
}

describe('markdownSerializer — byte-stable round-trip (REQ-W15 gating test)', () => {
  it('plain paragraph', () => {
    const md = 'Hello world.';
    expect(canonical(roundTrip(md))).toBe(canonical(md));
  });

  it('headings H1-H3', () => {
    const md = '# Heading 1\n\n## Heading 2\n\n### Heading 3';
    expect(canonical(roundTrip(md))).toBe(canonical(md));
  });

  it('bold and italic marks', () => {
    const md = 'This is **bold** and *italic* text.';
    expect(canonical(roundTrip(md))).toBe(canonical(md));
  });

  it('inline code', () => {
    const md = 'Use `cargo test` to run the tests.';
    expect(canonical(roundTrip(md))).toBe(canonical(md));
  });

  it('fenced code block', () => {
    const md = '```rust\nfn main() {\n    println!("Hello");\n}\n```';
    expect(canonical(roundTrip(md))).toBe(canonical(md));
  });

  it('fenced code block with language', () => {
    const md = '```typescript\nconst x = 42;\n```';
    expect(canonical(roundTrip(md))).toBe(canonical(md));
  });

  it('unordered list', () => {
    const md = '* Item one\n* Item two\n* Item three';
    expect(canonical(roundTrip(md))).toBe(canonical(md));
  });

  it('ordered list', () => {
    const md = '1. First\n2. Second\n3. Third';
    expect(canonical(roundTrip(md))).toBe(canonical(md));
  });

  it('blockquote', () => {
    const md = '> This is a quoted paragraph.';
    expect(canonical(roundTrip(md))).toBe(canonical(md));
  });

  it('horizontal rule', () => {
    const md = 'Before\n\n---\n\nAfter';
    expect(canonical(roundTrip(md))).toBe(canonical(md));
  });

  it('link', () => {
    const md = 'Visit [Atlas](https://atlas.example.com) today.';
    expect(canonical(roundTrip(md))).toBe(canonical(md));
  });

  it('wikilink preserved verbatim', () => {
    const md = 'See [[My Document]] for details.';
    expect(canonical(roundTrip(md))).toBe(canonical(md));
  });

  it('multiple wikilinks in one paragraph', () => {
    const md = 'From [[Source A]] to [[Destination B]].';
    expect(canonical(roundTrip(md))).toBe(canonical(md));
  });

  it('wikilink with special characters in title', () => {
    const md = 'Related: [[Guide: Getting Started]].';
    expect(canonical(roundTrip(md))).toBe(canonical(md));
  });

  it('mixed content: headings, lists, code, wikilinks', () => {
    const md = [
      '# Architecture Overview',
      '',
      'The system has three components:',
      '',
      '* **Frontend** — Vue 3 SPA (see [[Frontend Guide]])',
      '* **Backend** — Rust/axum (see [[Backend Guide]])',
      '* **Database** — Postgres 17',
      '',
      '## Setup',
      '',
      'Run `just dev` to start locally.',
      '',
      '```bash\njust db-up\njust dev\n```',
    ].join('\n');
    expect(canonical(roundTrip(md))).toBe(canonical(md));
  });

  it('strikethrough (if supported)', () => {
    const md = 'This is ~~struck~~ text.';
    const result = canonical(roundTrip(md));
    expect(result).toBe(canonical(md));
  });
});
