import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { nextTick } from 'vue';

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
  TrashPanel: { template: '<div data-stub="trash" />' },
  WorkspaceAuditPanel: { template: '<div data-stub="audit" />' },
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

  it.each([
    { isRoot: true },
    { isSystemAdmin: true },
  ])('renders Trash for a global admin direct route', (flags) => {
    const wrapper = mountView('trash', flags);

    expect(wrapper.find('[data-settings-row="trash"]').exists()).toBe(true);
    expect(wrapper.find('[data-stub="trash"]').exists()).toBe(true);
    expect(replace).not.toHaveBeenCalled();
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

  it('blocks a non-admin user from reaching Trash via the direct URL', () => {
    const wrapper = mountView('trash', {});

    expect(wrapper.find('[data-stub="trash"]').exists()).toBe(false);
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

  it('keeps a membership-gated section in the URL and hides its panel until the destination membership settles', async () => {
    const workspace = useWorkspaceStore();
    seedMembership('owner');
    let settleDestination: (() => void) | undefined;
    vi.spyOn(workspace, 'loadMembers').mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          settleDestination = () => {
            workspace.members = [
              { id: SELF_ID, display: 'u', principal_type: 'user', role: 'owner' } as PrincipalDto,
            ];
            resolve();
          };
        }),
    );

    const wrapper = mountView('audit');
    expect(wrapper.find('[data-stub="audit"]').exists()).toBe(true);

    workspace.setActiveWorkspace(null);
    await nextTick();
    expect(wrapper.find('[data-stub="audit"]').exists()).toBe(false);
    expect(replace).not.toHaveBeenCalled();

    workspace.setActiveWorkspace('bravo');
    await nextTick();
    expect(workspace.loadMembers).toHaveBeenCalledWith('bravo');
    expect(wrapper.find('[data-stub="audit"]').exists()).toBe(false);
    expect(replace).not.toHaveBeenCalled();

    settleDestination?.();
    await vi.waitFor(() => expect(wrapper.find('[data-stub="audit"]').exists()).toBe(true));
    expect(replace).not.toHaveBeenCalled();
  });

  it('applies the existing fallback only after the latest destination membership settles', async () => {
    const workspace = useWorkspaceStore();
    seedMembership('owner');
    let settleDestination: (() => void) | undefined;
    vi.spyOn(workspace, 'loadMembers').mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          settleDestination = () => resolve();
        }),
    );

    const wrapper = mountView('audit');
    workspace.setActiveWorkspace(null);
    await nextTick();
    workspace.setActiveWorkspace('bravo');
    await nextTick();

    expect(wrapper.find('[data-stub="account"]').exists()).toBe(false);
    expect(replace).not.toHaveBeenCalled();

    settleDestination?.();
    await vi.waitFor(() =>
      expect(replace).toHaveBeenCalledWith({ name: 'settings', params: { section: 'account' } }),
    );
    expect(wrapper.find('[data-stub="account"]').exists()).toBe(true);
  });

  it('ignores an older membership completion after a newer destination begins loading', async () => {
    const workspace = useWorkspaceStore();
    seedMembership('owner');
    const settleByWorkspace = new Map<string, () => void>();
    vi.spyOn(workspace, 'loadMembers').mockImplementation(
      (workspaceSlug) =>
        new Promise<void>((resolve) => {
          settleByWorkspace.set(workspaceSlug, () => {
            if (workspaceSlug === 'charlie') {
              workspace.members = [
                { id: SELF_ID, display: 'u', principal_type: 'user', role: 'owner' } as PrincipalDto,
              ];
            }
            resolve();
          });
        }),
    );

    const wrapper = mountView('audit');
    workspace.setActiveWorkspace(null);
    await nextTick();
    workspace.setActiveWorkspace('bravo');
    await nextTick();
    workspace.setActiveWorkspace('charlie');
    await nextTick();

    settleByWorkspace.get('bravo')?.();
    await nextTick();
    expect(wrapper.find('[data-stub="audit"]').exists()).toBe(false);
    expect(replace).not.toHaveBeenCalled();

    settleByWorkspace.get('charlie')?.();
    await vi.waitFor(() => expect(wrapper.find('[data-stub="audit"]').exists()).toBe(true));
    expect(replace).not.toHaveBeenCalled();
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
