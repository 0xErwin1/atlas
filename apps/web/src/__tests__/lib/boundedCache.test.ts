import { describe, expect, it } from 'vitest';
import { BoundedCache } from '@/lib/boundedCache';

describe('BoundedCache', () => {
  it('stores and returns values by key', () => {
    const cache = new BoundedCache<string>(2);
    cache.set('a', 'A');

    expect(cache.get('a')).toBe('A');
    expect(cache.get('missing')).toBeUndefined();
    expect(cache.size).toBe(1);
  });

  it('evicts the least recently used entry once the cap is exceeded', () => {
    const cache = new BoundedCache<string>(2);
    cache.set('a', 'A');
    cache.set('b', 'B');

    expect(cache.get('a')).toBe('A');

    cache.set('c', 'C');

    expect(cache.get('b')).toBeUndefined();
    expect(cache.get('a')).toBe('A');
    expect(cache.get('c')).toBe('C');
    expect(cache.size).toBe(2);
  });

  it('overwrites an existing key without growing', () => {
    const cache = new BoundedCache<string>(2);
    cache.set('a', 'A');
    cache.set('a', 'A2');

    expect(cache.get('a')).toBe('A2');
    expect(cache.size).toBe(1);
  });

  it('clears every entry', () => {
    const cache = new BoundedCache<string>(2);
    cache.set('a', 'A');
    cache.clear();

    expect(cache.get('a')).toBeUndefined();
    expect(cache.size).toBe(0);
  });
});
