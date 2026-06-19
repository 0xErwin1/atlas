import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it } from 'vitest';
import { useNotesTabsStore } from '@/stores/notesTabs';

function seed() {
  const store = useNotesTabsStore();
  store.open('ws', 'a', 'A');
  store.open('ws', 'b', 'B');
  store.open('ws', 'c', 'C');
  return store;
}

describe('useNotesTabsStore close actions', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    try {
      localStorage.clear();
    } catch {
      // jsdom provides localStorage; ignore if absent
    }
  });

  it('closeAll removes every tab and returns null', () => {
    const store = seed();
    expect(store.closeAll('ws')).toBeNull();
    expect(store.tabs('ws')).toEqual([]);
  });

  it('closeOthers keeps only the given tab', () => {
    const store = seed();
    expect(store.closeOthers('ws', 'b')).toBe('b');
    expect(store.tabs('ws').map((t) => t.slug)).toEqual(['b']);
  });

  it('closeRight removes only the tabs after the given one', () => {
    const store = seed();
    expect(store.closeRight('ws', 'b')).toBe('b');
    expect(store.tabs('ws').map((t) => t.slug)).toEqual(['a', 'b']);
  });

  it('close actions on an unknown slug are no-ops returning null', () => {
    const store = seed();
    expect(store.closeOthers('ws', 'zzz')).toBeNull();
    expect(store.closeRight('ws', 'zzz')).toBeNull();
    expect(store.tabs('ws')).toHaveLength(3);
  });
});
