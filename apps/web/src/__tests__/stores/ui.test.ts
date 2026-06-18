import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it } from 'vitest';
import { useUiStore } from '@/stores/ui';

describe('useUiStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    localStorage.clear();
  });

  it('inspector starts closed by default', () => {
    const store = useUiStore();
    expect(store.inspectorOpen).toBe(false);
  });

  it('toggleInspector opens when closed (REQ-W13)', () => {
    const store = useUiStore();
    store.toggleInspector();
    expect(store.inspectorOpen).toBe(true);
  });

  it('toggleInspector closes when open (REQ-W13)', () => {
    const store = useUiStore();
    store.inspectorOpen = true;
    store.toggleInspector();
    expect(store.inspectorOpen).toBe(false);
  });

  it('setInspectorTab changes tab while preserving open state (REQ-W13)', () => {
    const store = useUiStore();
    store.inspectorOpen = true;
    store.setInspectorTab('backlinks');
    expect(store.inspectorTab).toBe('backlinks');
    expect(store.inspectorOpen).toBe(true);
  });

  it('showBanner sets banner message and type', () => {
    const store = useUiStore();
    store.showBanner('Something went wrong', 'error');
    expect(store.banner).toEqual({ message: 'Something went wrong', type: 'error' });
  });

  it('dismissBanner clears banner', () => {
    const store = useUiStore();
    store.showBanner('oops', 'error');
    store.dismissBanner();
    expect(store.banner).toBeNull();
  });

  it('taskViewMode defaults to sidebar', () => {
    const store = useUiStore();
    expect(store.taskViewMode).toBe('sidebar');
  });

  it('setTaskViewMode changes the mode', () => {
    const store = useUiStore();
    store.setTaskViewMode('modal');
    expect(store.taskViewMode).toBe('modal');
  });

  it('setTaskViewMode persists the mode across store instances', () => {
    const first = useUiStore();
    first.setTaskViewMode('full');

    setActivePinia(createPinia());
    const second = useUiStore();
    expect(second.taskViewMode).toBe('full');
  });

  it('an unknown persisted mode falls back to sidebar', () => {
    localStorage.setItem('atlas.taskview.mode', 'bogus');
    const store = useUiStore();
    expect(store.taskViewMode).toBe('sidebar');
  });
});
