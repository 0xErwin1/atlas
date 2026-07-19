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

describe('useNotesTabsStore active tab', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    try {
      localStorage.clear();
    } catch {
      // jsdom provides localStorage; ignore if absent
    }
  });

  it('activeFor returns null until an active slug is set', () => {
    const store = seed();
    expect(store.activeFor('ws')).toBeNull();
  });

  it('setActive/activeFor round-trip the active slug per workspace', () => {
    const store = seed();
    store.setActive('ws', 'b');
    expect(store.activeFor('ws')).toBe('b');
    expect(store.activeFor('other')).toBeNull();
  });

  it('persists the active slug across a simulated reload', () => {
    const first = seed();
    first.setActive('ws', 'b');

    // A fresh Pinia instance rehydrates the store from localStorage, as on reload.
    setActivePinia(createPinia());
    const reloaded = useNotesTabsStore();
    expect(reloaded.activeFor('ws')).toBe('b');
  });

  it('moves the active slug to a neighbour when the active tab is closed', () => {
    const store = seed();
    store.setActive('ws', 'b');
    expect(store.close('ws', 'b')).toBe('c');
    expect(store.activeFor('ws')).toBe('c');
  });

  it('leaves the active slug untouched when a non-active tab is closed', () => {
    const store = seed();
    store.setActive('ws', 'b');
    store.close('ws', 'a');
    expect(store.activeFor('ws')).toBe('b');
  });

  it('clears the active slug when the last tab is closed', () => {
    const store = useNotesTabsStore();
    store.open('ws', 'a', 'A');
    store.setActive('ws', 'a');
    expect(store.close('ws', 'a')).toBeNull();
    expect(store.activeFor('ws')).toBeNull();
  });

  it('clears the active slug on closeAll', () => {
    const store = seed();
    store.setActive('ws', 'b');
    store.closeAll('ws');
    expect(store.activeFor('ws')).toBeNull();
  });

  it('falls back to the surviving tab when closeOthers drops the active tab', () => {
    const store = seed();
    store.setActive('ws', 'a');
    store.closeOthers('ws', 'c');
    expect(store.activeFor('ws')).toBe('c');
  });

  it('falls back to the anchor when closeRight drops the active tab', () => {
    const store = seed();
    store.setActive('ws', 'c');
    store.closeRight('ws', 'a');
    expect(store.activeFor('ws')).toBe('a');
  });

  it('keeps the active slug when closeRight does not drop it', () => {
    const store = seed();
    store.setActive('ws', 'a');
    store.closeRight('ws', 'b');
    expect(store.activeFor('ws')).toBe('a');
  });

  it('rehydrates without throwing from an old payload that lacks the active key', () => {
    localStorage.setItem('atlas:notes-tabs', JSON.stringify({ ws: [{ slug: 'a', title: 'A' }] }));

    setActivePinia(createPinia());
    const store = useNotesTabsStore();
    expect(store.tabs('ws').map((t) => t.slug)).toEqual(['a']);
    expect(store.activeFor('ws')).toBeNull();
  });
});
