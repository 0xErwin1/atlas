import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import ShortcutsHelpDialog from '@/components/shell/ShortcutsHelpDialog.vue';
import { getShortcutCatalog } from '@/lib/keymap';

function displayKey(token: string): string {
  return token.replace('mod', '⌘/Ctrl').replace('shift', 'Shift').replace('escape', 'Esc');
}

describe('ShortcutsHelpDialog', () => {
  it('lists the shared v1 shortcut catalog grouped for discovery', () => {
    const wrapper = mount(ShortcutsHelpDialog, {
      props: { open: true },
      global: { stubs: { Teleport: true } },
    });

    for (const shortcut of getShortcutCatalog()) {
      expect(wrapper.text()).toContain(shortcut.label);
      for (const key of shortcut.keys) {
        expect(wrapper.text().toLowerCase()).toContain(displayKey(key).toLowerCase());
      }
    }

    expect(wrapper.text()).toContain('Global');
    expect(wrapper.text()).toContain('Board');
  });
});
