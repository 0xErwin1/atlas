import { describe, expect, it } from 'vitest';
import { joinFrontmatter, splitFrontmatter } from '../../lib/frontmatter';

describe('splitFrontmatter', () => {
  it('splits a document with a YAML block into body and meta', () => {
    const raw = '---\ntitle: Hello\nstatus: draft\n---\n\nBody text here.';
    const { body, meta } = splitFrontmatter(raw);
    expect(body).toBe('\nBody text here.');
    expect(meta).toEqual({ title: 'Hello', status: 'draft' });
  });

  it('returns empty meta and full string as body when no frontmatter block', () => {
    const raw = 'Just body text.';
    const { body, meta } = splitFrontmatter(raw);
    expect(body).toBe('Just body text.');
    expect(meta).toEqual({});
  });

  it('returns empty meta when only --- is present but incomplete', () => {
    const raw = '---\ntitle: Test\n';
    const { body, meta } = splitFrontmatter(raw);
    expect(body).toBe('---\ntitle: Test\n');
    expect(meta).toEqual({});
  });

  it('handles empty frontmatter block', () => {
    const raw = '---\n---\n\nBody only.';
    const { body, meta } = splitFrontmatter(raw);
    expect(body).toBe('\nBody only.');
    expect(meta).toEqual({});
  });

  it('preserves nested YAML values in meta', () => {
    const raw = '---\ntitle: My Doc\ntags:\n  - rust\n  - web\n---\nContent.';
    const { meta } = splitFrontmatter(raw);
    expect(meta.title).toBe('My Doc');
    expect(meta.tags).toEqual(['rust', 'web']);
  });
});

describe('joinFrontmatter', () => {
  it('produces a document with YAML block when meta is non-empty', () => {
    const result = joinFrontmatter({ title: 'Hello', status: 'draft' }, 'Body text.');
    expect(result).toMatch(/^---\n/);
    expect(result).toContain('title: Hello');
    expect(result).toContain('status: draft');
    expect(result).toContain('---\n');
    expect(result.endsWith('Body text.')).toBe(true);
  });

  it('produces body only when meta is empty', () => {
    const result = joinFrontmatter({}, 'Body only.');
    expect(result).toBe('Body only.');
  });

  it('round-trip: splitFrontmatter(joinFrontmatter(meta, body)) reproduces meta and body', () => {
    const meta = { title: 'Test', draft: true };
    const body = '\nSome content here.';
    const joined = joinFrontmatter(meta, body);
    const { meta: outMeta, body: outBody } = splitFrontmatter(joined);
    expect(outMeta.title).toBe('Test');
    expect(outMeta.draft).toBe(true);
    expect(outBody).toBe(body);
  });

  it('round-trip: splitFrontmatter then joinFrontmatter produces the same string (no frontmatter)', () => {
    const raw = 'Plain document, no frontmatter.';
    const { meta, body } = splitFrontmatter(raw);
    const rejoined = joinFrontmatter(meta, body);
    expect(rejoined).toBe(raw);
  });

  it('round-trip: splitFrontmatter then joinFrontmatter preserves structure (with frontmatter)', () => {
    const raw = '---\ntitle: Atlas\nversion: 1\n---\n\nDocument body.';
    const { meta, body } = splitFrontmatter(raw);
    const rejoined = joinFrontmatter(meta, body);
    const { meta: meta2, body: body2 } = splitFrontmatter(rejoined);
    expect(meta2).toEqual(meta);
    expect(body2).toBe(body);
  });
});
