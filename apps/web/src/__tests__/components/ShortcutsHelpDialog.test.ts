import { mount } from '@vue/test-utils';
import { afterEach, describe, expect, it, vi } from 'vitest';
import ShortcutsHelpDialog from '@/components/shell/ShortcutsHelpDialog.vue';
import { formatShortcutKey, getShortcutCatalog } from '@/lib/keymap';

describe('ShortcutsHelpDialog', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('lists the shared v1 shortcut catalog grouped for discovery', () => {
    vi.stubGlobal('navigator', { ...navigator, platform: 'Linux x86_64' });

    const wrapper = mount(ShortcutsHelpDialog, {
      props: { open: true },
      global: { stubs: { Teleport: true } },
    });

    for (const shortcut of getShortcutCatalog()) {
      expect(wrapper.text()).toContain(shortcut.label);
      for (const key of shortcut.keys) {
        expect(wrapper.text().toLowerCase()).toContain(formatShortcutKey(key).toLowerCase());
      }
    }

    expect(wrapper.text()).toContain('Ctrl+K');
    expect(wrapper.text()).not.toContain('⌘');
    expect(wrapper.text()).toContain('Global');
    expect(wrapper.text()).toContain('Board');
  });
});
