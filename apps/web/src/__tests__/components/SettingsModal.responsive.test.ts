import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it } from 'vitest';
import SettingsModal from '@/components/settings/SettingsModal.vue';
import { useUiStore } from '@/stores/ui';

function setViewportWidth(width: number): void {
  Object.defineProperty(window, 'innerWidth', { value: width, configurable: true, writable: true });
  window.dispatchEvent(new Event('resize'));
}

const stubs = {
  AccountPanel: { template: '<div data-stub="account" />' },
  ApiKeysPanel: { template: '<div data-stub="keys" />' },
  UsersPanel: { template: '<div data-stub="users" />' },
  AboutPanel: { template: '<div data-stub="about" />' },
};

function mountModal() {
  const ui = useUiStore();
  ui.openSettings();
  return mount(SettingsModal, { global: { stubs } });
}

describe('SettingsModal responsive', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  it('shows the nav rail and content side by side on desktop', () => {
    setViewportWidth(1280);
    const wrapper = mountModal();

    expect(wrapper.find('.atl-settings-nav').exists()).toBe(true);
    expect(wrapper.find('[data-stub="account"]').exists()).toBe(true);
  });

  it('shows a sectioned list (no panel) on mobile before drilling in', () => {
    setViewportWidth(390);
    const wrapper = mountModal();

    expect(wrapper.text()).toContain('Account');
    expect(wrapper.text()).toContain('API keys');
    expect(wrapper.find('[data-stub="account"]').exists()).toBe(false);
    expect(wrapper.find('.atl-settings-nav').exists()).toBe(false);
  });

  it('drills into a panel with a back affordance on mobile', async () => {
    setViewportWidth(390);
    const wrapper = mountModal();

    await wrapper.find('[data-settings-row="keys"]').trigger('click');

    expect(wrapper.find('[data-stub="keys"]').exists()).toBe(true);
    expect(wrapper.find('[data-action="settings-back"]').exists()).toBe(true);
  });

  it('returns to the list from a panel on mobile', async () => {
    setViewportWidth(390);
    const wrapper = mountModal();

    await wrapper.find('[data-settings-row="keys"]').trigger('click');
    await wrapper.find('[data-action="settings-back"]').trigger('click');

    expect(wrapper.find('[data-stub="keys"]').exists()).toBe(false);
    expect(wrapper.find('[data-settings-row="account"]').exists()).toBe(true);
  });
});
