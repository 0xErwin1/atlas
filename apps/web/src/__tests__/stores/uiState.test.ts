import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const { GET, PUT } = vi.hoisted(() => ({ GET: vi.fn(), PUT: vi.fn() }));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET, PUT },
}));

import { useUiStateStore } from '@/stores/uiState';

describe('useUiStateStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('load reads expanded folders from the server state', async () => {
    GET.mockResolvedValue({ data: { state: { expandedFolders: ['f1'] } } });

    const store = useUiStateStore();
    await store.load();

    expect(store.loaded).toBe(true);
    expect(store.isFolderCollapsed('f1')).toBe(false);
    expect(store.isFolderCollapsed('f2')).toBe(true);
  });

  it('treats an empty state as all folders collapsed', async () => {
    GET.mockResolvedValue({ data: { state: {} } });

    const store = useUiStateStore();
    await store.load();

    expect(store.isFolderCollapsed('anything')).toBe(true);
  });

  it('setFolderCollapsed(false) expands and debounces a single PUT', () => {
    const store = useUiStateStore();

    store.setFolderCollapsed('f1', false);
    expect(store.isFolderCollapsed('f1')).toBe(false);

    store.setFolderCollapsed('f2', false);
    // Debounced: no PUT yet, then exactly one after the window.
    expect(PUT).not.toHaveBeenCalled();
    vi.advanceTimersByTime(600);

    expect(PUT).toHaveBeenCalledTimes(1);
    expect(PUT).toHaveBeenCalledWith('/v1/me/ui-state', {
      body: { state: { expandedFolders: ['f1', 'f2'] } },
    });
  });

  it('setFolderCollapsed(true) removes the folder from the expanded set', () => {
    const store = useUiStateStore();
    store.setFolderCollapsed('f1', false);
    store.setFolderCollapsed('f1', true);

    expect(store.isFolderCollapsed('f1')).toBe(true);
  });

  it('load reads persisted board views from the server state', async () => {
    GET.mockResolvedValue({ data: { state: { boardViews: { b1: 'list' } } } });

    const store = useUiStateStore();
    await store.load();

    expect(store.boardViewFor('b1')).toBe('list');
    expect(store.boardViewFor('b2')).toBeUndefined();
  });

  it('treats an empty state as no persisted board views', async () => {
    GET.mockResolvedValue({ data: { state: {} } });

    const store = useUiStateStore();
    await store.load();

    expect(store.boardViewFor('anything')).toBeUndefined();
  });

  it('setBoardView persists per-board and debounces a single PUT', () => {
    const store = useUiStateStore();

    store.setBoardView('b1', 'list');
    expect(store.boardViewFor('b1')).toBe('list');

    store.setBoardView('b2', 'table');
    // Debounced: no PUT yet, then exactly one after the window.
    expect(PUT).not.toHaveBeenCalled();
    vi.advanceTimersByTime(600);

    expect(PUT).toHaveBeenCalledTimes(1);
    expect(PUT).toHaveBeenCalledWith('/v1/me/ui-state', {
      body: { state: { boardViews: { b1: 'list', b2: 'table' } } },
    });
  });

  it('setBoardView keeps each board isolated and overwrites only the given board', () => {
    const store = useUiStateStore();

    store.setBoardView('b1', 'list');
    store.setBoardView('b2', 'table');
    store.setBoardView('b1', 'calendar');

    expect(store.boardViewFor('b1')).toBe('calendar');
    expect(store.boardViewFor('b2')).toBe('table');
  });
});
