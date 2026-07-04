import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { effectScope } from 'vue';
import { useCrossTabSync } from '@/composables/useCrossTabSync';
import { useLabelColorsStore } from '@/stores/labelColors';
import { useUiStore } from '@/stores/ui';

function dispatchStorage(key: string | null, newValue: string | null): void {
  window.dispatchEvent(new StorageEvent('storage', { key, newValue }));
}

describe('useCrossTabSync', () => {
  let scope: ReturnType<typeof effectScope>;

  beforeEach(() => {
    localStorage.clear();
    document.documentElement.removeAttribute('data-theme');
    setActivePinia(createPinia());

    scope = effectScope();
    scope.run(() => useCrossTabSync());
  });

  afterEach(() => {
    scope.stop();
    vi.restoreAllMocks();
  });

  it('applies an external theme change to state and the document root', () => {
    const ui = useUiStore();
    expect(ui.theme).toBe('dark');

    dispatchStorage('atlas:theme', 'light');

    expect(ui.theme).toBe('light');
    expect(document.documentElement.dataset.theme).toBe('light');
  });

  it('applies an external editor-wide change', () => {
    const ui = useUiStore();
    expect(ui.editorWide).toBe(false);

    dispatchStorage('atlas:editor-wide', '1');

    expect(ui.editorWide).toBe(true);
  });

  it('applies an external task-view-mode change', () => {
    const ui = useUiStore();
    expect(ui.taskViewMode).toBe('sidebar');

    dispatchStorage('atlas.taskview.mode', 'modal');

    expect(ui.taskViewMode).toBe('modal');
  });

  it('applies an external label-colors change', () => {
    const labelColors = useLabelColorsStore();

    dispatchStorage('atlas:label-colors', JSON.stringify({ 'tag:rust': 'amber' }));

    expect(labelColors.colors).toEqual({ 'tag:rust': 'amber' });
  });

  it('applies an external known-tags change', () => {
    const labelColors = useLabelColorsStore();

    dispatchStorage('atlas:known-tags', JSON.stringify(['rust', 'vue']));

    expect(labelColors.knownTags).toEqual(['rust', 'vue']);
  });

  it('ignores malformed payloads without throwing or mutating state', () => {
    const ui = useUiStore();
    const labelColors = useLabelColorsStore();

    dispatchStorage('atlas:theme', 'chartreuse');
    dispatchStorage('atlas.taskview.mode', 'nonsense');
    dispatchStorage('atlas:label-colors', 'not json');
    dispatchStorage('atlas:known-tags', '{ broken');

    expect(ui.theme).toBe('dark');
    expect(ui.taskViewMode).toBe('sidebar');
    expect(labelColors.colors).toEqual({});
    expect(labelColors.knownTags).toEqual([]);
  });

  it('ignores a null newValue (key removed)', () => {
    const ui = useUiStore();

    dispatchStorage('atlas:theme', null);

    expect(ui.theme).toBe('dark');
  });

  it('never writes back to localStorage when adopting an external value', () => {
    const setItem = vi.spyOn(Storage.prototype, 'setItem');

    dispatchStorage('atlas:theme', 'light');
    dispatchStorage('atlas:editor-wide', '1');
    dispatchStorage('atlas.taskview.mode', 'full');
    dispatchStorage('atlas:label-colors', JSON.stringify({ 'tag:go': 'teal' }));
    dispatchStorage('atlas:known-tags', JSON.stringify(['go']));

    expect(setItem).not.toHaveBeenCalled();
  });

  it('ignores keys outside the synced set', () => {
    const ui = useUiStore();
    const initialCollapsed = ui.sidebarCollapsed;

    dispatchStorage('atlas:sidebar-collapsed', '1');
    dispatchStorage('atlas:inspector', JSON.stringify({ open: true, tab: 'activity' }));

    expect(ui.sidebarCollapsed).toBe(initialCollapsed);
    expect(ui.inspectorOpen).toBe(false);
  });

  it('stops listening after the scope is disposed', () => {
    const ui = useUiStore();

    scope.stop();
    dispatchStorage('atlas:theme', 'light');

    expect(ui.theme).toBe('dark');
  });
});
