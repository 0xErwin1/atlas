import { type DOMWrapper, flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { nextTick } from 'vue';
import UsersPanel from '@/components/settings/UsersPanel.vue';
import { useAuthStore } from '@/stores/auth';
import { type UserDto, useUsersStore } from '@/stores/users';
import { useWorkspaceStore, type WorkspaceDto } from '@/stores/workspace';

// ConfirmDialog teleports to <body>, so its nodes live outside the wrapper.
function dialogEl<T extends Element = HTMLElement>(selector: string): T | null {
  return document.body.querySelector<T>(selector);
}

function clickDialog(selector: string): void {
  dialogEl(selector)?.dispatchEvent(new Event('click', { bubbles: true }));
}

// Each workspace row's role control is a Dropdown whose listbox teleports to
// <body>: open its trigger, then click the teleported option matching the label.
async function pickRole(row: DOMWrapper<Element>, label: string): Promise<void> {
  await row.find('button').trigger('click');
  await nextTick();

  const option = Array.from(document.body.querySelectorAll<HTMLElement>('li[role="option"]')).find(
    (li) => li.textContent?.trim() === label,
  );
  if (option === undefined) throw new Error(`option not found: ${label}`);

  option.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  await nextTick();
}

async function roleOptionLabels(row: DOMWrapper<Element>): Promise<string[]> {
  await row.find('button').trigger('click');
  await nextTick();
  return Array.from(document.body.querySelectorAll<HTMLElement>('li[role="option"]')).map(
    (li) => li.textContent?.trim() ?? '',
  );
}

function wsRowAt(wrapper: VueWrapper, index: number): DOMWrapper<Element> {
  const row = wrapper.findAll('[data-wsa-row]')[index];
  if (row === undefined) throw new Error(`workspace row not found: ${index}`);
  return row;
}

function user(over: Partial<UserDto> = {}): UserDto {
  return {
    id: 'u1',
    username: 'alice',
    display_name: 'Alice',
    is_root: false,
    is_system_admin: false,
    created_at: '2024-01-01T00:00:00Z',
    updated_at: '2024-01-01T00:00:00Z',
    activated_at: '2024-01-02T00:00:00Z',
    disabled_at: null,
    ...over,
  };
}

function workspace(slug: string, name: string): WorkspaceDto {
  return {
    id: `id-${slug}`,
    slug,
    name,
    created_at: '2024-01-01T00:00:00Z',
    updated_at: '2024-01-01T00:00:00Z',
  };
}

interface SetupOptions {
  root?: boolean;
  users?: UserDto[];
  memberships?: Record<string, Record<string, string>>;
}

function setup(opts: SetupOptions = {}) {
  const auth = useAuthStore();
  auth.user = { id: 'me', is_root: opts.root ?? true, is_system_admin: false } as never;

  const usersStore = useUsersStore();
  usersStore.users = opts.users ?? [user()];
  usersStore.memberships = {};
  vi.spyOn(usersStore, 'loadUsers').mockResolvedValue(undefined);
  vi.spyOn(usersStore, 'loadMemberships').mockImplementation(async (id: string) => {
    const map = opts.memberships?.[id] ?? {};
    usersStore.memberships = { ...usersStore.memberships, [id]: map };
    return map;
  });

  const wsStore = useWorkspaceStore();
  wsStore.adminWorkspaces = [workspace('acme', 'Acme'), workspace('beta', 'Beta')];
  vi.spyOn(wsStore, 'loadAdminWorkspaces').mockResolvedValue(undefined);

  return { usersStore, wsStore };
}

let activeWrapper: VueWrapper | null = null;

function mountPanel(): VueWrapper {
  const wrapper = mount(UsersPanel, { attachTo: document.body });
  activeWrapper = wrapper;
  return wrapper;
}

async function expandFirst(wrapper: VueWrapper): Promise<void> {
  await flushPromises();
  await wrapper.find('[data-action="manage"]').trigger('click');
  await flushPromises();
}

afterEach(() => {
  activeWrapper?.unmount();
  activeWrapper = null;
  document.body.innerHTML = '';
});

describe('UsersPanel — manage panel', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('expands a user row into the manage panel and loads memberships', async () => {
    const { usersStore } = setup({ memberships: { u1: { acme: 'admin' } } });

    const wrapper = mountPanel();
    expect(wrapper.find('[data-manage-panel]').exists()).toBe(false);

    await expandFirst(wrapper);

    expect(wrapper.find('[data-manage-panel]').exists()).toBe(true);
    expect(usersStore.loadMemberships).toHaveBeenCalledWith('u1');
  });

  it('shows the system-admin toggle for root viewing a non-root user and opens a confirm', async () => {
    setup();

    const wrapper = mountPanel();
    await expandFirst(wrapper);

    const toggle = wrapper.find('[data-action="toggle-sysadmin"]');
    expect(toggle.exists()).toBe(true);

    await toggle.trigger('click');

    expect(dialogEl('[data-test="confirm"]')).not.toBeNull();
    expect(document.body.textContent).toContain('Promote to system-admin?');
  });

  it('hides the system-admin toggle when the current user is not root', async () => {
    setup({ root: false });

    const wrapper = mountPanel();
    await expandFirst(wrapper);

    expect(wrapper.find('[data-action="toggle-sysadmin"]').exists()).toBe(false);
  });

  it('hides the system-admin toggle for a root target', async () => {
    setup({ users: [user({ is_root: true })] });

    const wrapper = mountPanel();
    await expandFirst(wrapper);

    expect(wrapper.find('[data-action="toggle-sysadmin"]').exists()).toBe(false);
  });

  it('reset password is a labeled button that opens a confirm', async () => {
    setup();

    const wrapper = mountPanel();
    await expandFirst(wrapper);

    const reset = wrapper.find('[data-action="reset-password"]');
    expect(reset.exists()).toBe(true);
    expect(reset.text()).toContain('Reset password');

    await reset.trigger('click');

    expect(document.body.textContent).toContain("Reset this user's password?");
  });
});

describe('UsersPanel — workspace access editor', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('lists admin workspaces and adds a member when assigning an unassigned workspace', async () => {
    const { wsStore } = setup({ memberships: { u1: { acme: 'admin' } } });
    const add = vi.spyOn(wsStore, 'addMember').mockResolvedValue(true);

    const wrapper = mountPanel();
    await expandFirst(wrapper);

    const rows = wrapper.findAll('[data-wsa-row]');
    expect(rows).toHaveLength(2);

    // Second row is Beta (unassigned). Assign it the member role.
    await pickRole(wsRowAt(wrapper, 1), 'Member');
    await flushPromises();

    expect(add).toHaveBeenCalledWith('beta', 'u1', 'member');
  });

  it('updates the role when changing an already-assigned workspace', async () => {
    const { wsStore } = setup({ memberships: { u1: { acme: 'admin' } } });
    const update = vi.spyOn(wsStore, 'updateMemberRole').mockResolvedValue(true);

    const wrapper = mountPanel();
    await expandFirst(wrapper);

    await pickRole(wsRowAt(wrapper, 0), 'Member');
    await flushPromises();

    expect(update).toHaveBeenCalledWith('acme', 'u1', 'member');
  });

  it('confirms then removes access when setting an assigned workspace to None', async () => {
    const { wsStore } = setup({ memberships: { u1: { acme: 'admin' } } });
    const remove = vi.spyOn(wsStore, 'removeMember').mockResolvedValue(true);

    const wrapper = mountPanel();
    await expandFirst(wrapper);

    await pickRole(wsRowAt(wrapper, 0), 'None');

    expect(remove).not.toHaveBeenCalled();

    clickDialog('[data-test="confirm"]');
    await flushPromises();

    expect(remove).toHaveBeenCalledWith('acme', 'u1');
  });

  it('offers the owner option only to root', async () => {
    setup({ root: true });

    const wrapper = mountPanel();
    await expandFirst(wrapper);

    const labels = await roleOptionLabels(wsRowAt(wrapper, 0));
    expect(labels).toContain('Owner');
    expect(labels).toContain('None');
  });
});
