import { flushPromises } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const { GET, PUT } = vi.hoisted(() => ({ GET: vi.fn(), PUT: vi.fn() }));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET, PUT },
}));

import { useUiStateStore } from '@/stores/uiState';

function deferredResponse() {
  let resolve: (value: { data: object; error: undefined }) => void = () => {};
  const promise = new Promise<{ data: object; error: undefined }>((resolver) => {
    resolve = resolver;
  });
  return { promise, resolve };
}

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
    expect(store.isFolderCollapsed('workspace', 'f1')).toBe(false);
    expect(store.isFolderCollapsed('workspace', 'f2')).toBe(true);
  });

  it('treats an empty state as all folders collapsed', async () => {
    GET.mockResolvedValue({ data: { state: {} } });

    const store = useUiStateStore();
    await store.load();

    expect(store.isFolderCollapsed('workspace', 'anything')).toBe(true);
  });

  it('setFolderCollapsed(false) expands and debounces a single PUT', () => {
    const store = useUiStateStore();

    store.setFolderCollapsed('workspace', 'f1', false);
    expect(store.isFolderCollapsed('workspace', 'f1')).toBe(false);

    store.setFolderCollapsed('workspace', 'f2', false);
    // Debounced: no PUT yet, then exactly one after the window.
    expect(PUT).not.toHaveBeenCalled();
    vi.advanceTimersByTime(600);

    expect(PUT).toHaveBeenCalledTimes(1);
    expect(PUT).toHaveBeenCalledWith('/api/me/ui-state', {
      body: {
        state: {
          sidebarExpansionByWorkspace: {
            workspace: { collapsedProjects: [], expandedFolders: ['f1', 'f2'] },
          },
        },
      },
    });
  });

  it('setFolderCollapsed(true) removes the folder from the expanded set', () => {
    const store = useUiStateStore();
    store.setFolderCollapsed('workspace', 'f1', false);
    store.setFolderCollapsed('workspace', 'f1', true);

    expect(store.isFolderCollapsed('workspace', 'f1')).toBe(true);
  });

  it('scopes folder and project expansion by workspace with the expected defaults', () => {
    const store = useUiStateStore();

    expect(store.isProjectCollapsed('workspace-a', 'project-1')).toBe(false);
    expect(store.isFolderCollapsed('workspace-a', 'folder-1')).toBe(true);

    store.setProjectCollapsed('workspace-a', 'project-1', true);
    store.setFolderCollapsed('workspace-a', 'folder-1', false);

    expect(store.isProjectCollapsed('workspace-a', 'project-1')).toBe(true);
    expect(store.isProjectCollapsed('workspace-b', 'project-1')).toBe(false);
    expect(store.isFolderCollapsed('workspace-a', 'folder-1')).toBe(false);
    expect(store.isFolderCollapsed('workspace-b', 'folder-1')).toBe(true);
  });

  it('reads legacy expandedFolders until a workspace-specific preference is written', async () => {
    GET.mockResolvedValue({ data: { state: { expandedFolders: ['legacy-folder'] } } });
    const store = useUiStateStore();
    await store.load();

    expect(store.isFolderCollapsed('workspace-a', 'legacy-folder')).toBe(false);

    store.setFolderCollapsed('workspace-a', 'workspace-folder', false);

    expect(store.isFolderCollapsed('workspace-a', 'legacy-folder')).toBe(false);
    expect(store.isFolderCollapsed('workspace-a', 'workspace-folder')).toBe(false);
    expect(store.isFolderCollapsed('workspace-b', 'workspace-folder')).toBe(true);
  });

  it('reset cancels pending writes and clears the loaded identity state', () => {
    const store = useUiStateStore();
    store.setProjectCollapsed('workspace-a', 'project-1', true);

    store.reset();
    vi.advanceTimersByTime(600);

    expect(store.data).toEqual({});
    expect(store.loaded).toBe(false);
    expect(PUT).not.toHaveBeenCalled();
  });

  it('surfaces a failed preference write instead of discarding it', async () => {
    PUT.mockResolvedValue({ error: { hint: 'Preferences unavailable' } });
    const store = useUiStateStore();

    store.setProjectCollapsed('workspace-a', 'project-1', true);
    await vi.advanceTimersByTimeAsync(600);

    expect(store.error).toBe('Preferences unavailable');
  });

  it('serializes writes and sends the latest snapshot after an in-flight PUT', async () => {
    const first = deferredResponse();
    const second = deferredResponse();
    PUT.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    const store = useUiStateStore();

    store.setProjectCollapsed('workspace-a', 'project-1', true);
    await vi.advanceTimersByTimeAsync(600);
    expect(PUT).toHaveBeenCalledTimes(1);

    store.setFolderCollapsed('workspace-a', 'folder-1', false);
    await vi.advanceTimersByTimeAsync(600);
    expect(PUT).toHaveBeenCalledTimes(1);

    first.resolve({ data: {}, error: undefined });
    await flushPromises();

    expect(PUT).toHaveBeenCalledTimes(2);
    expect(PUT).toHaveBeenLastCalledWith('/api/me/ui-state', {
      body: {
        state: {
          sidebarExpansionByWorkspace: {
            'workspace-a': {
              collapsedProjects: ['project-1'],
              expandedFolders: ['folder-1'],
            },
          },
        },
      },
    });

    second.resolve({ data: {}, error: undefined });
  });

  it('never queues a prior identity snapshot after reset while a PUT is in flight', async () => {
    const prior = deferredResponse();
    PUT.mockReturnValueOnce(prior.promise).mockResolvedValueOnce({ data: {}, error: undefined });
    const store = useUiStateStore();

    store.setProjectCollapsed('workspace-a', 'prior-project', true);
    await vi.advanceTimersByTimeAsync(600);
    expect(PUT).toHaveBeenCalledTimes(1);

    store.reset();
    store.setDefaultBoardView('list');
    await vi.advanceTimersByTimeAsync(600);
    expect(PUT).toHaveBeenCalledTimes(1);

    prior.resolve({ data: {}, error: undefined });
    await flushPromises();

    expect(PUT).toHaveBeenCalledTimes(2);
    expect(PUT).toHaveBeenLastCalledWith('/api/me/ui-state', {
      body: { state: { defaultBoardView: 'list' } },
    });
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
    expect(PUT).toHaveBeenCalledWith('/api/me/ui-state', {
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

  it('load reads the pinned default board view from the server state', async () => {
    GET.mockResolvedValue({ data: { state: { defaultBoardView: 'timeline' } } });

    const store = useUiStateStore();
    await store.load();

    expect(store.defaultBoardView()).toBe('timeline');
  });

  it('treats an empty or unknown default board view as no preference', async () => {
    GET.mockResolvedValue({ data: { state: { defaultBoardView: 'gantt' } } });

    const store = useUiStateStore();
    await store.load();

    expect(store.defaultBoardView()).toBeNull();
  });

  it('setDefaultBoardView persists the pinned layout', () => {
    const store = useUiStateStore();

    store.setDefaultBoardView('list');
    expect(store.defaultBoardView()).toBe('list');

    vi.advanceTimersByTime(600);

    expect(PUT).toHaveBeenCalledTimes(1);
    expect(PUT).toHaveBeenCalledWith('/api/me/ui-state', {
      body: { state: { defaultBoardView: 'list' } },
    });
  });

  it('setDefaultBoardView(null) clears the preference without touching per-board views', () => {
    const store = useUiStateStore();

    store.setBoardView('b1', 'table');
    store.setDefaultBoardView('list');
    store.setDefaultBoardView(null);

    expect(store.defaultBoardView()).toBeNull();
    expect(store.boardViewFor('b1')).toBe('table');

    vi.advanceTimersByTime(600);

    expect(PUT).toHaveBeenCalledWith('/api/me/ui-state', {
      body: { state: { boardViews: { b1: 'table' } } },
    });
  });
});
