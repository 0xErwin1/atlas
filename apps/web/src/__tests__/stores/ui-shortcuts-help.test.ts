import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it } from 'vitest';
import { useUiStore } from '@/stores/ui';

describe('useUiStore shortcuts help', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    localStorage.clear();
  });

  it('starts with shortcuts help closed', () => {
    const store = useUiStore();
    expect(store.shortcutsHelpOpen).toBe(false);
  });

  it('opens and closes shortcuts help', () => {
    const store = useUiStore();

    store.openShortcutsHelp();
    expect(store.shortcutsHelpOpen).toBe(true);

    store.closeShortcutsHelp();
    expect(store.shortcutsHelpOpen).toBe(false);
  });
});
