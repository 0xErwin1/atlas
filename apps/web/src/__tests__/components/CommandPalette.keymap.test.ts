import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import CommandPalette from '@/components/search/CommandPalette.vue';
import { installKeymapListener, resetKeymapForTests } from '@/composables/useKeymap';

const { GET } = vi.hoisted(() => ({
  GET: vi.fn(),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET },
}));

describe('CommandPalette keymap integration', () => {
  let uninstall: (() => void) | null = null;

  beforeEach(() => {
    setActivePinia(createPinia());
    resetKeymapForTests();
    uninstall = installKeymapListener();
    GET.mockResolvedValue({ data: { items: [], next_cursor: null, has_more: false }, error: undefined });
  });

  afterEach(() => {
    uninstall?.();
    resetKeymapForTests();
    vi.clearAllMocks();
  });

  it('closes on overlay-priority Escape when focus is not in the search input', async () => {
    const wrapper = mount(CommandPalette, {
      props: { ws: 'acme', open: true, actions: [] },
      attachTo: document.body,
    });

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await wrapper.vm.$nextTick();

    expect(wrapper.emitted('close')).toBeTruthy();
    wrapper.unmount();
  });
});
