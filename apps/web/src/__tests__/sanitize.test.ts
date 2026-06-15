import { describe, expect, it } from 'vitest';
import { sanitizeSnippet } from '../lib/sanitize';

describe('sanitizeSnippet (REQ-W25)', () => {
  it('strips <script> tags', () => {
    const result = sanitizeSnippet('<script>alert(1)</script>');
    expect(result).not.toContain('<script>');
    expect(result).not.toContain('</script>');
  });

  it('preserves <mark> tags', () => {
    const result = sanitizeSnippet('hello <mark>world</mark> foo');
    expect(result).toBe('hello <mark>world</mark> foo');
  });

  it('strips <img> with onerror XSS', () => {
    const result = sanitizeSnippet('<img src=x onerror="alert(1)">');
    expect(result).not.toContain('<img');
  });

  it('strips <a> tags but keeps text content', () => {
    const result = sanitizeSnippet('<a href="x">link text</a>');
    expect(result).toContain('link text');
    expect(result).not.toContain('<a');
    expect(result).not.toContain('</a>');
  });

  it('handles mixed allowed and disallowed tags', () => {
    const result = sanitizeSnippet('see <b>this</b> <mark>highlighted</mark> item');
    expect(result).toBe('see this <mark>highlighted</mark> item');
  });

  it('keeps plain text unchanged', () => {
    expect(sanitizeSnippet('plain text')).toBe('plain text');
  });
});
