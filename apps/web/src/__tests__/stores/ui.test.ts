import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
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

  it('showBanner auto-dismisses after its type timeout', () => {
    vi.useFakeTimers();
    try {
      const store = useUiStore();
      store.showBanner('Saved', 'success');
      expect(store.banner).not.toBeNull();

      vi.advanceTimersByTime(3999);
      expect(store.banner).not.toBeNull();

      vi.advanceTimersByTime(1);
      expect(store.banner).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it('showBanner resets the dismiss timer for a replacing toast', () => {
    vi.useFakeTimers();
    try {
      const store = useUiStore();
      store.showBanner('First', 'success');
      vi.advanceTimersByTime(3000);

      store.showBanner('Second', 'success');
      // The first toast's 4s deadline passes, but the replacement restarted it.
      vi.advanceTimersByTime(1500);
      expect(store.banner).toEqual({ message: 'Second', type: 'success' });

      vi.advanceTimersByTime(2500);
      expect(store.banner).toBeNull();
    } finally {
      vi.useRealTimers();
    }
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
