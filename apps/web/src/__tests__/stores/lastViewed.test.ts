import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it } from 'vitest';
import { useLastViewedStore } from '@/stores/lastViewed';

const STORAGE_KEY = 'atlas:last-viewed';

describe('useLastViewedStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    try {
      localStorage.clear();
    } catch {
      // jsdom provides localStorage; ignore if absent
    }
  });

  it('records a target per workspace and reads it back', () => {
    const store = useLastViewedStore();

    store.record('atlas', { name: 'notes', params: { slug: 'roadmap' } });
    store.record('personal', { name: 'task-detail', params: { readableId: 'ATL-9' } });

    expect(store.forWorkspace('atlas')).toEqual({ name: 'notes', params: { slug: 'roadmap' } });
    expect(store.forWorkspace('personal')).toEqual({
      name: 'task-detail',
      params: { readableId: 'ATL-9' },
    });
  });

  it('returns null for a workspace with no history', () => {
    const store = useLastViewedStore();
    expect(store.forWorkspace('unknown')).toBeNull();
  });

  it('overwrites the previous target for the same workspace', () => {
    const store = useLastViewedStore();

    store.record('atlas', { name: 'notes', params: { slug: 'a' } });
    store.record('atlas', { name: 'notes', params: { slug: 'b' } });

    expect(store.forWorkspace('atlas')).toEqual({ name: 'notes', params: { slug: 'b' } });
  });

  it('clears a workspace entry so it is no longer restored', () => {
    const store = useLastViewedStore();
    store.record('atlas', { name: 'notes', params: { slug: 'gone' } });

    store.clear('atlas');

    expect(store.forWorkspace('atlas')).toBeNull();
  });

  it('clearIfMatches drops the entry when the stored target matches', () => {
    const store = useLastViewedStore();
    store.record('atlas', { name: 'notes', params: { slug: 'gone' } });

    store.clearIfMatches('atlas', { name: 'notes', params: { slug: 'gone' } });

    expect(store.forWorkspace('atlas')).toBeNull();
  });

  it('clearIfMatches keeps the entry when the route name differs', () => {
    const store = useLastViewedStore();
    store.record('atlas', { name: 'notes', params: { slug: 'keep' } });

    store.clearIfMatches('atlas', { name: 'tasks', params: { slug: 'keep' } });

    expect(store.forWorkspace('atlas')).toEqual({ name: 'notes', params: { slug: 'keep' } });
  });

  it('clearIfMatches keeps the entry when the params differ', () => {
    const store = useLastViewedStore();
    store.record('atlas', { name: 'notes', params: { slug: 'keep' } });

    store.clearIfMatches('atlas', { name: 'notes', params: { slug: 'other' } });

    expect(store.forWorkspace('atlas')).toEqual({ name: 'notes', params: { slug: 'keep' } });
  });

  it('persists to localStorage and rehydrates on a fresh store', () => {
    const store = useLastViewedStore();
    store.record('atlas', { name: 'tasks', params: { boardId: 'board-1' } });

    const raw = localStorage.getItem(STORAGE_KEY);
    expect(raw).not.toBeNull();
    expect(JSON.parse(raw as string)).toEqual({
      atlas: { name: 'tasks', params: { boardId: 'board-1' } },
    });

    setActivePinia(createPinia());
    const rehydrated = useLastViewedStore();
    expect(rehydrated.forWorkspace('atlas')).toEqual({
      name: 'tasks',
      params: { boardId: 'board-1' },
    });
  });

  it('starts empty when stored JSON is malformed', () => {
    localStorage.setItem(STORAGE_KEY, '{not json');

    const store = useLastViewedStore();
    expect(store.forWorkspace('atlas')).toBeNull();
  });

  it('starts empty when stored JSON is the literal null', () => {
    localStorage.setItem(STORAGE_KEY, 'null');

    const store = useLastViewedStore();
    expect(store.forWorkspace('atlas')).toBeNull();
  });

  it('starts empty when stored JSON is an array', () => {
    localStorage.setItem(STORAGE_KEY, '[{"name":"notes"}]');

    const store = useLastViewedStore();
    expect(store.forWorkspace('0')).toBeNull();
    expect(store.forWorkspace('atlas')).toBeNull();
  });
});
