import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { isDesktop, push, replace, routeParams } = vi.hoisted(() => ({
  isDesktop: { value: false },
  push: vi.fn(),
  replace: vi.fn(),
  routeParams: { value: {} as Record<string, string> },
}));

vi.mock('vue-router', () => ({
  useRoute: () => ({ params: routeParams.value }),
  useRouter: () => ({ push, replace }),
}));

vi.mock('@/platform/transport', () => ({
  getPlatformTransport: () => ({ isDesktop: isDesktop.value }),
}));

import { type MeResponse, useAuthStore } from '@/stores/auth';
import { type PrincipalDto, useWorkspaceStore } from '@/stores/workspace';
import SettingsView from '@/views/SettingsView.vue';

const stubs = {
  AppShell: {
    template: '<div><slot name="sidebar" /><slot /></div>',
  },
  AccountPanel: { template: '<div data-stub="account" />' },
  ApiKeysPanel: { template: '<div data-stub="keys" />' },
  UsersPanel: { template: '<div data-stub="users" />' },
  AboutPanel: { template: '<div data-stub="about" />' },
  AppSettingsPanel: { template: '<div data-stub="app" />' },
};

const SELF_ID = '00000000-0000-0000-0000-000000000001';

interface UserFlags {
  isRoot?: boolean;
  isSystemAdmin?: boolean;
}

function seedUser(flags: UserFlags): void {
  const auth = useAuthStore();
  auth.user = {
    id: SELF_ID,
    username: 'u',
    is_root: flags.isRoot ?? false,
    is_system_admin: flags.isSystemAdmin ?? false,
  } as MeResponse;
}

function seedMembership(role: 'owner' | 'admin' | 'member'): void {
  const wsStore = useWorkspaceStore();
  wsStore.activeWorkspaceSlug = 'acme';
  wsStore.members = [{ id: SELF_ID, display: 'u', principal_type: 'user', role } as PrincipalDto];
}

function mountView(section: string | undefined, flags: UserFlags = {}) {
  routeParams.value = section === undefined ? {} : { section };
  seedUser(flags);
  return mount(SettingsView, { global: { stubs } });
}

describe('SettingsView routing', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    isDesktop.value = false;
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
    const wrapper = mountView('users', { isRoot: true });

    expect(wrapper.find('[data-settings-row="users"]').exists()).toBe(true);
    expect(wrapper.find('[data-settings-row="about"]').exists()).toBe(true);
    expect(wrapper.find('[data-stub="users"]').exists()).toBe(true);
  });

  it('shows administration sections to a system admin', () => {
    const wrapper = mountView('users', { isSystemAdmin: true });

    expect(wrapper.find('[data-settings-row="users"]').exists()).toBe(true);
    expect(wrapper.find('[data-settings-row="platform-audit"]').exists()).toBe(true);
    expect(wrapper.find('[data-stub="users"]').exists()).toBe(true);
  });

  it('hides administration sections from a non-admin user', () => {
    const wrapper = mountView('account', {});

    expect(wrapper.find('[data-settings-row="users"]').exists()).toBe(false);
    expect(wrapper.find('[data-settings-row="about"]').exists()).toBe(false);
    expect(wrapper.find('[data-settings-row="platform-audit"]').exists()).toBe(false);
  });

  it('blocks a non-admin user from reaching a root-only section via the URL', () => {
    const wrapper = mountView('users', {});

    expect(wrapper.find('[data-stub="users"]').exists()).toBe(false);
    expect(wrapper.find('[data-stub="account"]').exists()).toBe(true);
    expect(replace).toHaveBeenCalledWith({ name: 'settings', params: { section: 'account' } });
  });

  it('hides the security log from a plain member', () => {
    seedMembership('member');
    const wrapper = mountView('account', {});

    expect(wrapper.find('[data-settings-row="audit"]').exists()).toBe(false);
  });

  it('shows the security log to a workspace owner', () => {
    seedMembership('owner');
    const wrapper = mountView('account', {});

    expect(wrapper.find('[data-settings-row="audit"]').exists()).toBe(true);
  });

  it('shows the security log to a workspace admin', () => {
    seedMembership('admin');
    const wrapper = mountView('account', {});

    expect(wrapper.find('[data-settings-row="audit"]').exists()).toBe(true);
  });

  it('shows the security log to a system admin with no membership here', () => {
    const wrapper = mountView('account', { isSystemAdmin: true });

    expect(wrapper.find('[data-settings-row="audit"]').exists()).toBe(true);
  });

  it('redirects a plain member away from the audit section reached via URL', () => {
    seedMembership('member');
    const wrapper = mountView('audit', {});

    expect(wrapper.find('[data-settings-row="audit"]').exists()).toBe(false);
    expect(replace).toHaveBeenCalledWith({ name: 'settings', params: { section: 'account' } });
  });

  it('hides the app settings section on the web build', () => {
    const wrapper = mountView('account');

    expect(wrapper.find('[data-settings-row="app"]').exists()).toBe(false);
    expect(wrapper.find('[data-stub="app"]').exists()).toBe(false);
  });

  it('redirects the web build away from the app settings section reached via URL', () => {
    const wrapper = mountView('app');

    expect(wrapper.find('[data-stub="app"]').exists()).toBe(false);
    expect(wrapper.find('[data-stub="account"]').exists()).toBe(true);
    expect(replace).toHaveBeenCalledWith({ name: 'settings', params: { section: 'account' } });
  });

  it('shows the app settings section on the desktop build and routes to its panel', () => {
    isDesktop.value = true;
    const wrapper = mountView('app');

    expect(wrapper.find('[data-settings-row="app"]').exists()).toBe(true);
    expect(wrapper.find('[data-stub="app"]').exists()).toBe(true);
  });

  it('navigates when a section is clicked', async () => {
    const wrapper = mountView('account');

    await wrapper.find('[data-settings-row="keys"]').trigger('click');

    expect(push).toHaveBeenCalledWith({ name: 'settings', params: { section: 'keys' } });
  });
});
