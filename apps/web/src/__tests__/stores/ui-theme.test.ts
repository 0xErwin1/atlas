import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it } from 'vitest';
import { useUiStore } from '@/stores/ui';

describe('ui store — theme', () => {
  beforeEach(() => {
    localStorage.clear();
    document.documentElement.removeAttribute('data-theme');
    setActivePinia(createPinia());
  });

  it('defaults to dark and applies it to the document root', () => {
    const ui = useUiStore();

    expect(ui.theme).toBe('dark');
    expect(document.documentElement.dataset.theme).toBe('dark');
  });

  it('setTheme switches the document attribute and persists the choice', () => {
    const ui = useUiStore();

    ui.setTheme('light');

    expect(ui.theme).toBe('light');
    expect(document.documentElement.dataset.theme).toBe('light');
    expect(localStorage.getItem('atlas:theme')).toBe('light');
  });

  it('restores a persisted theme on initialisation', () => {
    localStorage.setItem('atlas:theme', 'light');
    setActivePinia(createPinia());

    const ui = useUiStore();

    expect(ui.theme).toBe('light');
    expect(document.documentElement.dataset.theme).toBe('light');
  });
});
