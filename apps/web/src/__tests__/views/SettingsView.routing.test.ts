import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { push, replace, routeParams } = vi.hoisted(() => ({
  push: vi.fn(),
  replace: vi.fn(),
  routeParams: { value: {} as Record<string, string> },
}));

vi.mock('vue-router', () => ({
  useRoute: () => ({ params: routeParams.value }),
  useRouter: () => ({ push, replace }),
}));

import { type MeResponse, useAuthStore } from '@/stores/auth';
import SettingsView from '@/views/SettingsView.vue';

const stubs = {
  AppShell: {
    template: '<div><slot name="sidebar" /><slot /></div>',
  },
  AccountPanel: { template: '<div data-stub="account" />' },
  ApiKeysPanel: { template: '<div data-stub="keys" />' },
  UsersPanel: { template: '<div data-stub="users" />' },
  AboutPanel: { template: '<div data-stub="about" />' },
};

function seedUser(isRoot: boolean): void {
  const auth = useAuthStore();
  auth.user = { username: 'u', is_root: isRoot } as MeResponse;
}

function mountView(section: string | undefined, isRoot = false) {
  routeParams.value = section === undefined ? {} : { section };
  seedUser(isRoot);
  return mount(SettingsView, { global: { stubs } });
}

describe('SettingsView routing', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    push.mockClear();
    replace.mockClear();
  });

  it('renders the Account panel for the account section', () => {
    const wrapper = mountView('account');

    expect(wrapper.find('[data-stub="account"]').exists()).toBe(true);
    expect(wrapper.find('[data-stub="keys"]').exists()).toBe(false);
  });

  it('renders the API keys panel for the keys section', () => {
    const wrapper = mountView('keys');

    expect(wrapper.find('[data-stub="keys"]').exists()).toBe(true);
  });

  it('falls back to the Account panel when no section is given', () => {
    const wrapper = mountView(undefined);

    expect(wrapper.find('[data-stub="account"]').exists()).toBe(true);
    expect(replace).toHaveBeenCalledWith({ name: 'settings', params: { section: 'account' } });
  });

  it('falls back to the Account panel for an unknown section', () => {
    const wrapper = mountView('does-not-exist');

    expect(wrapper.find('[data-stub="account"]').exists()).toBe(true);
    expect(replace).toHaveBeenCalledWith({ name: 'settings', params: { section: 'account' } });
  });

  it('shows administration sections to a root user', () => {
    const wrapper = mountView('users', true);

    expect(wrapper.find('[data-settings-row="users"]').exists()).toBe(true);
    expect(wrapper.find('[data-settings-row="about"]').exists()).toBe(true);
    expect(wrapper.find('[data-stub="users"]').exists()).toBe(true);
  });

  it('hides administration sections from a non-root user', () => {
    const wrapper = mountView('account', false);

    expect(wrapper.find('[data-settings-row="users"]').exists()).toBe(false);
    expect(wrapper.find('[data-settings-row="about"]').exists()).toBe(false);
  });

  it('blocks a non-root user from reaching a root-only section via the URL', () => {
    const wrapper = mountView('users', false);

    expect(wrapper.find('[data-stub="users"]').exists()).toBe(false);
    expect(wrapper.find('[data-stub="account"]').exists()).toBe(true);
    expect(replace).toHaveBeenCalledWith({ name: 'settings', params: { section: 'account' } });
  });

  it('navigates when a section is clicked', async () => {
    const wrapper = mountView('account');

    await wrapper.find('[data-settings-row="keys"]').trigger('click');

    expect(push).toHaveBeenCalledWith({ name: 'settings', params: { section: 'keys' } });
  });
});
