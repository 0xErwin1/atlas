import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import App from '@/App.vue';
import { resetKeymapForTests } from '@/composables/useKeymap';
import { useUiStore } from '@/stores/ui';

const push = vi.fn();

vi.mock('vue-router', async () => {
  const actual = await vi.importActual<typeof import('vue-router')>('vue-router');
  return {
    ...actual,
    useRouter: () => ({ push }),
  };
});

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET: vi.fn() },
}));

describe('App keymap wiring', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    resetKeymapForTests();
    push.mockReset();
  });

  afterEach(() => {
    resetKeymapForTests();
  });

  it('toggles the command palette through the global keymap shortcut', async () => {
    const wrapper = mount(App, {
      global: { stubs: { RouterView: true, Teleport: true } },
    });
    const ui = useUiStore();

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', metaKey: true, bubbles: true }));
    await wrapper.vm.$nextTick();
    expect(ui.paletteOpen).toBe(true);

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', ctrlKey: true, bubbles: true }));
    await wrapper.vm.$nextTick();
    expect(ui.paletteOpen).toBe(false);

    wrapper.unmount();
  });

  it('opens shortcuts help from Shift+? and closes it with Escape before lower-priority shortcuts', async () => {
    const wrapper = mount(App, {
      global: { stubs: { RouterView: true, Teleport: true } },
    });
    const ui = useUiStore();

    window.dispatchEvent(new KeyboardEvent('keydown', { key: '?', shiftKey: true, bubbles: true }));
    await wrapper.vm.$nextTick();
    expect(ui.shortcutsHelpOpen).toBe(true);

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await wrapper.vm.$nextTick();
    expect(ui.shortcutsHelpOpen).toBe(false);

    wrapper.unmount();
  });

  it('opens shortcuts help from the command palette action', async () => {
    const wrapper = mount(App, {
      global: { stubs: { RouterView: true, Teleport: true } },
    });
    const ui = useUiStore();

    const palette = wrapper.getComponent({ name: 'CommandPalette' });
    palette.vm.$emit('select', {
      type: 'action',
      action: { id: 'show-shortcuts-help', label: 'Show keyboard shortcuts', kind: 'action' },
    });
    await wrapper.vm.$nextTick();

    expect(ui.shortcutsHelpOpen).toBe(true);
    wrapper.unmount();
  });
});
