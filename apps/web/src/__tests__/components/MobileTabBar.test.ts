import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const push = vi.fn();
const currentRoute = { name: 'notes' as string };

vi.mock('vue-router', () => ({
  useRouter: () => ({ push }),
  useRoute: () => currentRoute,
}));

import MobileTabBar from '@/components/shell/MobileTabBar.vue';

describe('MobileTabBar', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    push.mockClear();
    currentRoute.name = 'notes';
  });

  it('renders the four bottom tabs', () => {
    const wrapper = mount(MobileTabBar);

    for (const tab of ['notes', 'tasks', 'search', 'more']) {
      expect(wrapper.find(`[data-tab="${tab}"]`).exists()).toBe(true);
    }
  });

  it('marks the active tab from the current route', () => {
    currentRoute.name = 'tasks';
    const wrapper = mount(MobileTabBar);

    expect(wrapper.find('[data-tab="tasks"]').attributes('aria-current')).toBe('page');
    expect(wrapper.find('[data-tab="notes"]').attributes('aria-current')).toBeUndefined();
  });

  it('navigates when a nav tab is clicked', async () => {
    const wrapper = mount(MobileTabBar);

    await wrapper.find('[data-tab="search"]').trigger('click');

    expect(push).toHaveBeenCalledWith({ name: 'search' });
  });

  it('opens the More sheet without navigating', async () => {
    const wrapper = mount(MobileTabBar);

    expect(wrapper.text()).not.toContain('Log out');

    await wrapper.find('[data-tab="more"]').trigger('click');

    expect(push).not.toHaveBeenCalled();
    expect(wrapper.text()).toContain('Log out');
    expect(wrapper.text()).toContain('Settings');
  });
});
