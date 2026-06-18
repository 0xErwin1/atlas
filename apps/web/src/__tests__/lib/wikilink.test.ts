import { describe, expect, it } from 'vitest';
import {
  detectWikilinkTrigger,
  filterWikilinkCandidates,
  formatWikilink,
  parseWikilinkInner,
  wikilinkHref,
} from '@/lib/wikilink';

describe('detectWikilinkTrigger', () => {
  it('detects a trigger with the partial query after [[', () => {
    const trigger = detectWikilinkTrigger('see [[Arch', 10);

    expect(trigger).not.toBeNull();
    expect(trigger?.query).toBe('Arch');
    expect(trigger?.from).toBe(4);
  });

  it('returns an empty query right after the opening brackets', () => {
    const trigger = detectWikilinkTrigger('[[', 2);

    expect(trigger?.query).toBe('');
    expect(trigger?.from).toBe(0);
  });

  it('returns null when there is no [[', () => {
    expect(detectWikilinkTrigger('plain text', 10)).toBeNull();
  });

  it('returns null when the link is already closed', () => {
    expect(detectWikilinkTrigger('[[Done]]', 8)).toBeNull();
  });

  it('returns null when a newline interrupts the query', () => {
    expect(detectWikilinkTrigger('[[multi\nline', 12)).toBeNull();
  });
});

describe('filterWikilinkCandidates', () => {
  const candidates = [{ title: 'Architecture' }, { title: 'Roadmap' }, { title: 'API design' }];

  it('returns all candidates for an empty query', () => {
    expect(filterWikilinkCandidates(candidates, '')).toHaveLength(3);
  });

  it('filters by case-insensitive substring', () => {
    const result = filterWikilinkCandidates(candidates, 'a');
    expect(result.map((c) => c.title)).toEqual(['Architecture', 'Roadmap', 'API design']);
  });

  it('narrows to a single match', () => {
    const result = filterWikilinkCandidates(candidates, 'road');
    expect(result.map((c) => c.title)).toEqual(['Roadmap']);
  });
});

const UUID = '019ed5fa-6df7-7201-97ce-a99abae541c1';

describe('parseWikilinkInner', () => {
  it('parses an id-bound link into the stable id and display title', () => {
    expect(parseWikilinkInner(`${UUID}|Editor test`)).toEqual({ id: UUID, title: 'Editor test' });
  });

  it('trims surrounding whitespace around id and title', () => {
    expect(parseWikilinkInner(`  ${UUID} | Editor test `)).toEqual({ id: UUID, title: 'Editor test' });
  });

  it('treats a plain title as a title-only link', () => {
    expect(parseWikilinkInner('API Design')).toEqual({ id: null, title: 'API Design' });
  });

  it('treats a non-uuid before the pipe as a legacy title', () => {
    expect(parseWikilinkInner('Foo|Bar')).toEqual({ id: null, title: 'Foo|Bar' });
  });
});

describe('formatWikilink', () => {
  it('serializes an id-bound ref with the pipe', () => {
    expect(formatWikilink({ id: UUID, title: 'Editor test' })).toBe(`[[${UUID}|Editor test]]`);
  });

  it('serializes a title-only ref without a pipe', () => {
    expect(formatWikilink({ id: null, title: 'Roadmap' })).toBe('[[Roadmap]]');
  });
});

describe('wikilinkHref', () => {
  it('navigates an id-bound ref by the stable uuid', () => {
    expect(wikilinkHref({ id: UUID, title: 'Whatever the title is now' })).toBe(`/n/${UUID}`);
  });

  it('falls back to the slugified title for a title-only ref', () => {
    expect(wikilinkHref({ id: null, title: 'API Design' })).toBe('/n/api-design');
  });
});
