import { describe, expect, it, vi } from 'vitest';
import { collectPaged } from '@/lib/pagination';

describe('collectPaged', () => {
  it('returns a single page when there is no more', async () => {
    const fetchPage = vi.fn().mockResolvedValue({ data: { items: [1, 2], has_more: false } });

    const { items, error } = await collectPaged(fetchPage);

    expect(items).toEqual([1, 2]);
    expect(error).toBeUndefined();
    expect(fetchPage).toHaveBeenCalledOnce();
    expect(fetchPage).toHaveBeenCalledWith(undefined);
  });

  it('follows the cursor and accumulates every page', async () => {
    const fetchPage = vi
      .fn()
      .mockResolvedValueOnce({ data: { items: ['a'], next_cursor: 'c1', has_more: true } })
      .mockResolvedValueOnce({ data: { items: ['b'], next_cursor: 'c2', has_more: true } })
      .mockResolvedValueOnce({ data: { items: ['c'], next_cursor: null, has_more: false } });

    const { items, error } = await collectPaged(fetchPage);

    expect(items).toEqual(['a', 'b', 'c']);
    expect(error).toBeUndefined();
    expect(fetchPage).toHaveBeenCalledTimes(3);
    expect(fetchPage).toHaveBeenNthCalledWith(2, 'c1');
    expect(fetchPage).toHaveBeenNthCalledWith(3, 'c2');
  });

  it('stops on error, returning items collected so far and the error', async () => {
    const fetchPage = vi
      .fn()
      .mockResolvedValueOnce({ data: { items: ['a'], next_cursor: 'c1', has_more: true } })
      .mockResolvedValueOnce({ error: { hint: 'boom' } });

    const { items, error } = await collectPaged(fetchPage);

    expect(items).toEqual(['a']);
    expect(error).toEqual({ hint: 'boom' });
  });

  it('treats has_more true with a null cursor as the end', async () => {
    const fetchPage = vi.fn().mockResolvedValue({ data: { items: [1], next_cursor: null, has_more: true } });

    const { items } = await collectPaged(fetchPage);

    expect(items).toEqual([1]);
    expect(fetchPage).toHaveBeenCalledOnce();
  });
});
