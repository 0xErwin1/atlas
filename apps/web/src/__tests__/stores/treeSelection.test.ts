import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it } from 'vitest';
import { useTreeSelection } from '@/stores/treeSelection';

describe('useTreeSelection', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('selectOnly replaces the selection and sets the anchor', () => {
    const s = useTreeSelection();
    s.selectOnly('doc:a');
    s.selectOnly('doc:b');

    expect(s.keys()).toEqual(['doc:b']);
    expect(s.count).toBe(1);
    expect(s.anchor).toBe('doc:b');
  });

  it('toggle adds and removes a key without dropping the rest', () => {
    const s = useTreeSelection();
    s.selectOnly('doc:a');
    s.toggle('doc:b');
    expect(new Set(s.keys())).toEqual(new Set(['doc:a', 'doc:b']));

    s.toggle('doc:a');
    expect(s.keys()).toEqual(['doc:b']);
  });

  it('selectRange selects everything between the anchor and the target', () => {
    const s = useTreeSelection();
    s.setOrder(['folder:f1', 'doc:a', 'doc:b', 'doc:c', 'doc:d']);
    s.selectOnly('doc:a');
    s.selectRange('doc:c');

    expect(s.keys()).toEqual(['doc:a', 'doc:b', 'doc:c']);
  });

  it('selectRange works upward from the anchor too', () => {
    const s = useTreeSelection();
    s.setOrder(['doc:a', 'doc:b', 'doc:c', 'doc:d']);
    s.selectOnly('doc:d');
    s.selectRange('doc:b');

    expect(s.keys()).toEqual(['doc:b', 'doc:c', 'doc:d']);
  });

  it('activate maps modifiers to selection behaviour', () => {
    const s = useTreeSelection();
    s.setOrder(['doc:a', 'doc:b', 'doc:c']);

    expect(s.activate('doc:a', {})).toBe('default');
    expect(s.activate('doc:b', { meta: true })).toBe('selection-only');
    expect(new Set(s.keys())).toEqual(new Set(['doc:a', 'doc:b']));

    expect(s.activate('doc:c', { shift: true })).toBe('selection-only');
    // shift extends from the anchor (doc:b, set by the meta-click).
    expect(s.keys()).toEqual(['doc:b', 'doc:c']);
  });
});
