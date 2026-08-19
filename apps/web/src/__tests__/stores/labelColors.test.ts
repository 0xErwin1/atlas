import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it } from 'vitest';
import { defaultSwatchId } from '@/lib/swatches';
import { useLabelColorsStore } from '@/stores/labelColors';

describe('labelColors store — tagColor', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    localStorage.clear();
  });

  it('resolves a label name through its case-insensitive tag key', () => {
    const store = useLabelColorsStore();
    store.setColor('tag:bug', 'red');

    expect(store.tagColor('bug')).toBe('red');
    expect(store.tagColor('Bug')).toBe('red');
    expect(store.tagColor('BUG')).toBe('red');
  });

  it('falls back to the deterministic default for an unchosen label', () => {
    const store = useLabelColorsStore();

    expect(store.tagColor('Needs-Review')).toBe(defaultSwatchId('tag:needs-review'));
  });

  it('matches colorFor on the same key', () => {
    const store = useLabelColorsStore();

    expect(store.tagColor('Backend')).toBe(store.colorFor('tag:backend'));
  });
});
