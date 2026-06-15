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

  it('handles unicode characters by replacing them', () => {
    expect(slugify('Café au lait')).toBe('caf-au-lait');
  });

  it('handles empty string', () => {
    expect(slugify('')).toBe('');
  });

  it('handles all non-alphanumeric input', () => {
    expect(slugify('---!!!')).toBe('');
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
});
