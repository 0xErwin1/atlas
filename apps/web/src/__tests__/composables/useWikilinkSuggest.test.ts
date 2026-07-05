import { describe, expect, it, vi } from 'vitest';
import { useWikilinkSuggest } from '@/composables/useWikilinkSuggest';

function makeSuggest(open: boolean) {
  return {
    open,
    moveDown: vi.fn(),
    moveUp: vi.fn(),
    confirmActive: vi.fn(),
  };
}

describe('useWikilinkSuggest', () => {
  it('tracks the active query and caret from onQuery', () => {
    const { query, caret, onQuery } = useWikilinkSuggest(
      () => null,
      () => null,
    );

    onQuery('foo', { left: 10, top: 20 });
    expect(query.value).toBe('foo');
    expect(caret.value).toEqual({ left: 10, top: 20 });

    onQuery(null, null);
    expect(query.value).toBeNull();
    expect(caret.value).toBeNull();
  });

  it('inserts the chosen reference and clears the trigger on select', () => {
    const insertWikilink = vi.fn();
    const { query, caret, onQuery, onSelect } = useWikilinkSuggest(
      () => ({ insertWikilink }),
      () => null,
    );

    onQuery('bar', { left: 1, top: 2 });
    onSelect({ id: 'doc-1', title: 'Bar' });

    expect(insertWikilink).toHaveBeenCalledWith({ id: 'doc-1', title: 'Bar' });
    expect(query.value).toBeNull();
    expect(caret.value).toBeNull();
  });

  it('ignores navigation keys while the suggestion list is closed', () => {
    const suggest = makeSuggest(false);
    const { onKeydown } = useWikilinkSuggest(
      () => null,
      () => suggest,
    );

    const event = new KeyboardEvent('keydown', { key: 'ArrowDown', cancelable: true });
    onKeydown(event);

    expect(suggest.moveDown).not.toHaveBeenCalled();
    expect(event.defaultPrevented).toBe(false);
  });

  it('routes arrow, enter and escape to the open suggestion list', () => {
    const suggest = makeSuggest(true);
    const { query, caret, onQuery, onKeydown } = useWikilinkSuggest(
      () => null,
      () => suggest,
    );

    const down = new KeyboardEvent('keydown', { key: 'ArrowDown', cancelable: true });
    onKeydown(down);
    expect(suggest.moveDown).toHaveBeenCalledTimes(1);
    expect(down.defaultPrevented).toBe(true);

    onKeydown(new KeyboardEvent('keydown', { key: 'ArrowUp', cancelable: true }));
    expect(suggest.moveUp).toHaveBeenCalledTimes(1);

    onKeydown(new KeyboardEvent('keydown', { key: 'Enter', cancelable: true }));
    expect(suggest.confirmActive).toHaveBeenCalledTimes(1);

    onQuery('x', { left: 0, top: 0 });
    onKeydown(new KeyboardEvent('keydown', { key: 'Escape', cancelable: true }));
    expect(query.value).toBeNull();
    expect(caret.value).toBeNull();
  });
});
