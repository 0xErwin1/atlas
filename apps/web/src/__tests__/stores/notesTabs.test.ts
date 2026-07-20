import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it } from 'vitest';
import { parseTabKey, type TabRef, tabKey, useNotesTabsStore } from '@/stores/notesTabs';

const doc = (id: string): TabRef => ({ kind: 'doc', id });
const board = (id: string): TabRef => ({ kind: 'board', id });

function seed() {
  const store = useNotesTabsStore();
  store.open('ws', doc('a'), 'A');
  store.open('ws', doc('b'), 'B');
  store.open('ws', doc('c'), 'C');
  return store;
}

function clearStorage() {
  try {
    localStorage.clear();
  } catch {
    // jsdom provides localStorage; ignore if absent
  }
}

describe('useNotesTabsStore close actions', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    clearStorage();
  });

  it('closeAll removes every tab and returns null', () => {
    const store = seed();
    expect(store.closeAll('ws')).toBeNull();
    expect(store.tabs('ws')).toEqual([]);
  });

  it('closeOthers keeps only the given tab', () => {
    const store = seed();
    expect(store.closeOthers('ws', doc('b'))).toEqual(doc('b'));
    expect(store.tabs('ws').map((t) => t.id)).toEqual(['b']);
  });

  it('closeRight removes only the tabs after the given one', () => {
    const store = seed();
    expect(store.closeRight('ws', doc('b'))).toEqual(doc('b'));
    expect(store.tabs('ws').map((t) => t.id)).toEqual(['a', 'b']);
  });

  it('close actions on an unknown tab are no-ops returning null', () => {
    const store = seed();
    expect(store.closeOthers('ws', doc('zzz'))).toBeNull();
    expect(store.closeRight('ws', doc('zzz'))).toBeNull();
    expect(store.tabs('ws')).toHaveLength(3);
  });

  it('close returns the right neighbour, else the left', () => {
    const store = seed();
    expect(store.close('ws', doc('b'))).toEqual(doc('c'));
    expect(store.close('ws', doc('c'))).toEqual(doc('a'));
  });
});

describe('useNotesTabsStore typed tabs and boards', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    clearStorage();
  });

  it('keeps a doc and a board with the same id as distinct tabs', () => {
    const store = useNotesTabsStore();
    store.open('ws', doc('x'), 'Doc X');
    store.open('ws', board('x'), 'Board X');

    expect(store.tabs('ws')).toEqual([
      { kind: 'doc', id: 'x', title: 'Doc X' },
      { kind: 'board', id: 'x', title: 'Board X' },
    ]);
  });

  it('closing the board leaves the same-id doc untouched', () => {
    const store = useNotesTabsStore();
    store.open('ws', doc('x'), 'Doc X');
    store.open('ws', board('x'), 'Board X');

    store.close('ws', board('x'));
    expect(store.tabs('ws')).toEqual([{ kind: 'doc', id: 'x', title: 'Doc X' }]);
  });

  it('round-trips a tab ref through its collision-free key', () => {
    expect(tabKey(board('b1'))).toBe('board:b1');
    expect(parseTabKey('board:b1')).toEqual(board('b1'));
    expect(parseTabKey(tabKey(doc('a:b')))).toEqual(doc('a:b'));
  });
});

describe('useNotesTabsStore active tab', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    clearStorage();
  });

  it('activeFor returns null until an active tab is set', () => {
    const store = seed();
    expect(store.activeFor('ws')).toBeNull();
  });

  it('setActive/activeFor round-trip the active ref per workspace', () => {
    const store = seed();
    store.setActive('ws', doc('b'));
    expect(store.activeFor('ws')).toEqual(doc('b'));
    expect(store.activeFor('other')).toBeNull();
  });

  it('persists the active ref across a simulated reload', () => {
    const first = seed();
    first.setActive('ws', doc('b'));

    setActivePinia(createPinia());
    const reloaded = useNotesTabsStore();
    expect(reloaded.activeFor('ws')).toEqual(doc('b'));
  });

  it('tracks a board tab as active', () => {
    const store = useNotesTabsStore();
    store.open('ws', board('b1'), 'Board');
    store.setActive('ws', board('b1'));
    expect(store.activeFor('ws')).toEqual(board('b1'));
  });

  it('moves the active ref to a neighbour when the active tab is closed', () => {
    const store = seed();
    store.setActive('ws', doc('b'));
    expect(store.close('ws', doc('b'))).toEqual(doc('c'));
    expect(store.activeFor('ws')).toEqual(doc('c'));
  });

  it('leaves the active ref untouched when a non-active tab is closed', () => {
    const store = seed();
    store.setActive('ws', doc('b'));
    store.close('ws', doc('a'));
    expect(store.activeFor('ws')).toEqual(doc('b'));
  });

  it('clears the active ref when the last tab is closed', () => {
    const store = useNotesTabsStore();
    store.open('ws', doc('a'), 'A');
    store.setActive('ws', doc('a'));
    expect(store.close('ws', doc('a'))).toBeNull();
    expect(store.activeFor('ws')).toBeNull();
  });

  it('clears the active ref on closeAll', () => {
    const store = seed();
    store.setActive('ws', doc('b'));
    store.closeAll('ws');
    expect(store.activeFor('ws')).toBeNull();
  });

  it('falls back to the surviving tab when closeOthers drops the active tab', () => {
    const store = seed();
    store.setActive('ws', doc('a'));
    store.closeOthers('ws', doc('c'));
    expect(store.activeFor('ws')).toEqual(doc('c'));
  });

  it('falls back to the anchor when closeRight drops the active tab', () => {
    const store = seed();
    store.setActive('ws', doc('c'));
    store.closeRight('ws', doc('a'));
    expect(store.activeFor('ws')).toEqual(doc('a'));
  });
});

describe('useNotesTabsStore dirty document tracking', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    clearStorage();
  });

  it('flags and clears the dirty document per workspace', () => {
    const store = useNotesTabsStore();
    expect(store.isDirtyDoc('ws', 'a')).toBe(false);

    store.setDirtyDoc('ws', 'a');
    expect(store.isDirtyDoc('ws', 'a')).toBe(true);
    expect(store.isDirtyDoc('ws', 'b')).toBe(false);

    store.setDirtyDoc('ws', null);
    expect(store.isDirtyDoc('ws', 'a')).toBe(false);
  });
});

describe('useNotesTabsStore legacy migration', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    clearStorage();
  });

  it('migrates old slug-only tab entries into typed doc tabs', () => {
    localStorage.setItem(
      'atlas:notes-tabs',
      JSON.stringify({
        ws: [
          { slug: 'a', title: 'A' },
          { slug: 'b', title: 'B' },
        ],
      }),
    );

    setActivePinia(createPinia());
    const store = useNotesTabsStore();
    expect(store.tabs('ws')).toEqual([
      { kind: 'doc', id: 'a', title: 'A' },
      { kind: 'doc', id: 'b', title: 'B' },
    ]);
  });

  it('migrates an old bare-slug active pointer into a doc ref', () => {
    localStorage.setItem('atlas:notes-tabs', JSON.stringify({ ws: [{ slug: 'a', title: 'A' }] }));
    localStorage.setItem('atlas:notes-active-tab', JSON.stringify({ ws: 'a' }));

    setActivePinia(createPinia());
    const store = useNotesTabsStore();
    expect(store.activeFor('ws')).toEqual({ kind: 'doc', id: 'a' });
  });

  it('rehydrates without throwing from an old payload that lacks the active key', () => {
    localStorage.setItem('atlas:notes-tabs', JSON.stringify({ ws: [{ slug: 'a', title: 'A' }] }));

    setActivePinia(createPinia());
    const store = useNotesTabsStore();
    expect(store.tabs('ws').map((t) => t.id)).toEqual(['a']);
    expect(store.activeFor('ws')).toBeNull();
  });

  it('reads back a typed payload it persisted', () => {
    const first = useNotesTabsStore();
    first.open('ws', board('b1'), 'Board');
    first.setActive('ws', board('b1'));

    setActivePinia(createPinia());
    const store = useNotesTabsStore();
    expect(store.tabs('ws')).toEqual([{ kind: 'board', id: 'b1', title: 'Board' }]);
    expect(store.activeFor('ws')).toEqual(board('b1'));
  });
});
