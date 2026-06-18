import { mount } from '@vue/test-utils';
import { describe, expect, it, vi } from 'vitest';
import ContextMenu, { type MenuItem } from '@/components/ui/ContextMenu.vue';

function mountMenu(items: MenuItem[]) {
  return mount(ContextMenu, {
    props: { open: true, x: 10, y: 10, items },
    global: { stubs: { teleport: true } },
  });
}

describe('ContextMenu submenus', () => {
  it('hides a submenu until its parent row is hovered, then reveals the children', async () => {
    const child = vi.fn();
    const wrapper = mountMenu([{ label: 'Change status', children: [{ label: 'Todo', action: child }] }]);

    expect(wrapper.find('.atl-submenu').exists()).toBe(false);

    await wrapper.find('.atl-mi-wrap').trigger('mouseenter');

    const submenu = wrapper.find('.atl-submenu');
    expect(submenu.exists()).toBe(true);
    expect(submenu.text()).toContain('Todo');
  });

  it('runs a child action and closes the menu when a submenu item is clicked', async () => {
    const child = vi.fn();
    const wrapper = mountMenu([{ label: 'Change status', children: [{ label: 'Todo', action: child }] }]);

    await wrapper.find('.atl-mi-wrap').trigger('mouseenter');
    await wrapper.find('.atl-submenu .atl-mi').trigger('click');

    expect(child).toHaveBeenCalledTimes(1);
    expect(wrapper.emitted('close')).toBeTruthy();
  });

  it('does not run a parent action or close when the parent has children', async () => {
    const parent = vi.fn();
    const wrapper = mountMenu([{ label: 'Change status', action: parent, children: [{ label: 'Todo' }] }]);

    // The parent row is the first .atl-mi (inside the wrap), clicking it is a no-op.
    await wrapper.find('.atl-mi-wrap .atl-mi').trigger('click');

    expect(parent).not.toHaveBeenCalled();
    expect(wrapper.emitted('close')).toBeFalsy();
  });
});
