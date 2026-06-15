import { describe, expect, it } from 'vitest';
import { slugify } from '../../lib/slugify';

describe('slugify', () => {
  it('lowercases a simple title', () => {
    expect(slugify('Hello World')).toBe('hello-world');
  });

  it('replaces non-alphanumeric characters with hyphens', () => {
    expect(slugify('Hello, World!')).toBe('hello-world');
  });

  it('collapses multiple consecutive hyphens', () => {
    expect(slugify('foo  --  bar')).toBe('foo-bar');
  });

  it('trims leading and trailing hyphens', () => {
    expect(slugify('  -- hello --  ')).toBe('hello');
  });

  it('preserves unicode alphanumeric characters (server parity)', () => {
    expect(slugify('Café au lait')).toBe('café-au-lait');
  });

  it('returns untitled for empty string (server parity)', () => {
    expect(slugify('')).toBe('untitled');
  });

  it('returns untitled for all non-alphanumeric input (server parity)', () => {
    expect(slugify('---!!!')).toBe('untitled');
  });

  it('preserves numbers in the slug', () => {
    expect(slugify('Article 42: The Answer')).toBe('article-42-the-answer');
  });

  it('handles leading numbers', () => {
    expect(slugify('123 Test')).toBe('123-test');
  });

  it('matches server-side behaviour: single space between words', () => {
    expect(slugify('My   Document  Title')).toBe('my-document-title');
  });

  it('truncates to 80 characters', () => {
    const long = 'a'.repeat(100);
    expect(slugify(long).length).toBe(80);
  });

  it('does not leave a trailing hyphen after truncation', () => {
    const title = `${'a'.repeat(79)} ${'b'.repeat(10)}`;
    const result = slugify(title);
    expect(result.endsWith('-')).toBe(false);
    expect(result.length).toBeLessThanOrEqual(80);
  });

  it('handles unicode titles with accented characters', () => {
    expect(slugify('Über die Brücke')).toBe('über-die-brücke');
  });
});
