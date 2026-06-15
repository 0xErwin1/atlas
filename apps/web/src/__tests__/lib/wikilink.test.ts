import { describe, expect, it } from 'vitest';
import { detectWikilinkTrigger, filterWikilinkCandidates, wikilinkTarget } from '@/lib/wikilink';

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

describe('wikilinkTarget', () => {
  it('resolves a title to the slugified note route', () => {
    expect(wikilinkTarget('API Design')).toBe('/n/api-design');
  });

  it('uses server-parity slugify for unicode titles', () => {
    expect(wikilinkTarget('Café Notes')).toBe('/n/café-notes');
  });
});
