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

  describe('editor reading and view mode', () => {
    it('default to editing in live mode', () => {
      const store = useUiStore();
      expect(store.editorReading).toBe(false);
      expect(store.editorMode).toBe('live');
    });

    // A board taking over the router outlet unmounts the notes view, so the
    // choice only survives the round trip if it outlives the store instance.
    it('survive a fresh store instance', () => {
      const first = useUiStore();
      first.toggleEditorReading();
      first.toggleEditorMode();

      setActivePinia(createPinia());
      const second = useUiStore();

      expect(second.editorReading).toBe(true);
      expect(second.editorMode).toBe('source');
    });

    it('persist a v-model write from the editor, not only a toggle', () => {
      const first = useUiStore();
      first.setEditorReading(true);
      first.setEditorMode('source');

      setActivePinia(createPinia());
      const second = useUiStore();

      expect(second.editorReading).toBe(true);
      expect(second.editorMode).toBe('source');
    });

    it('fall back to live editing on an unreadable persisted value', () => {
      localStorage.setItem('atlas:editor-reading', 'bogus');
      localStorage.setItem('atlas:editor-mode', 'bogus');
      const store = useUiStore();

      expect(store.editorReading).toBe(false);
      expect(store.editorMode).toBe('live');
    });

    it('mirror another tab without re-persisting', () => {
      const store = useUiStore();
      store.applyExternalEditorReading('1');
      store.applyExternalEditorMode('source');

      expect(store.editorReading).toBe(true);
      expect(store.editorMode).toBe('source');
      expect(localStorage.getItem('atlas:editor-reading')).toBeNull();
      expect(localStorage.getItem('atlas:editor-mode')).toBeNull();
    });
  });

  describe('editor line numbers', () => {
    // On by default: notes are addressed by line elsewhere in Atlas, and the
    // numbers those refer to are invisible without the gutter.
    it('default to on and survive a fresh store instance once turned off', () => {
      const first = useUiStore();
      expect(first.editorLineNumbers).toBe(true);

      first.toggleEditorLineNumbers();
      setActivePinia(createPinia());

      expect(useUiStore().editorLineNumbers).toBe(false);
    });

    it('mirrors another tab without re-persisting', () => {
      const store = useUiStore();
      store.applyExternalEditorLineNumbers('0');

      expect(store.editorLineNumbers).toBe(false);
      expect(localStorage.getItem('atlas:editor-line-numbers')).toBeNull();
    });
  });
});
