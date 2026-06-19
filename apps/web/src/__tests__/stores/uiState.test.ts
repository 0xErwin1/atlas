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

  it('load reads collapsed folders from the server state', async () => {
    GET.mockResolvedValue({ data: { state: { collapsedFolders: ['f1'] } } });

    const store = useUiStateStore();
    await store.load();

    expect(store.loaded).toBe(true);
    expect(store.isFolderCollapsed('f1')).toBe(true);
    expect(store.isFolderCollapsed('f2')).toBe(false);
  });

  it('treats an empty state as all folders expanded', async () => {
    GET.mockResolvedValue({ data: { state: {} } });

    const store = useUiStateStore();
    await store.load();

    expect(store.isFolderCollapsed('anything')).toBe(false);
  });

  it('setFolderCollapsed toggles state and debounces a single PUT', () => {
    const store = useUiStateStore();

    store.setFolderCollapsed('f1', true);
    expect(store.isFolderCollapsed('f1')).toBe(true);

    store.setFolderCollapsed('f2', true);
    // Debounced: no PUT yet, then exactly one after the window.
    expect(PUT).not.toHaveBeenCalled();
    vi.advanceTimersByTime(600);

    expect(PUT).toHaveBeenCalledTimes(1);
    expect(PUT).toHaveBeenCalledWith('/v1/me/ui-state', {
      body: { state: { collapsedFolders: ['f1', 'f2'] } },
    });
  });

  it('setFolderCollapsed(false) removes the folder from the collapsed set', () => {
    const store = useUiStateStore();
    store.setFolderCollapsed('f1', true);
    store.setFolderCollapsed('f1', false);

    expect(store.isFolderCollapsed('f1')).toBe(false);
  });
});
