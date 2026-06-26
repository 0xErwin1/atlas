import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { nextTick } from 'vue';
import MembersPanel from '@/components/settings/MembersPanel.vue';
import Dropdown from '@/components/ui/Dropdown.vue';
import { useAuthStore } from '@/stores/auth';
import { type PrincipalDto, type UserDto, useWorkspaceStore } from '@/stores/workspace';

// The add-member dialog is teleported to <body>, so its nodes live outside the
// component wrapper and must be queried through the document.
function dialogEl<T extends Element = HTMLElement>(selector: string): T | null {
  return document.body.querySelector<T>(selector);
}

function dialogAll(selector: string): HTMLElement[] {
  return Array.from(document.body.querySelectorAll<HTMLElement>(selector));
}

// The dialog role control is a Dropdown teleported with the dialog to <body>;
// drive it through the document with native events.
async function openRoleDropdown(): Promise<void> {
  dialogEl('[data-add-role] button')?.dispatchEvent(new Event('click', { bubbles: true }));
  await nextTick();
}

function roleOptionLabels(): string[] {
  return dialogAll('[data-add-role] li[role="option"]').map((li) => li.textContent?.trim() ?? '');
}

async function pickRole(label: string): Promise<void> {
  await openRoleDropdown();
  const option = dialogAll('[data-add-role] li[role="option"]').find(
    (li) => li.textContent?.trim() === label,
  );
  option?.dispatchEvent(new Event('click', { bubbles: true }));
  await nextTick();
}

function userMember(id: string, display: string, role: string): PrincipalDto {
  return { id, display, principal_type: 'user', role, account_status: 'active' };
}

function agentMember(id: string, display: string, keyType: string): PrincipalDto {
  return { id, display, principal_type: 'api_key', key_type: keyType };
}

function assignable(id: string, username: string, displayName: string, activated: boolean): UserDto {
  return {
    id,
    username,
    display_name: displayName,
    is_root: false,
    is_system_admin: false,
    created_at: '2024-01-01T00:00:00Z',
    updated_at: '2024-01-01T00:00:00Z',
    activated_at: activated ? '2024-01-01T00:00:00Z' : null,
  };
}

/**
 * Wires the auth user as a workspace member at the given role so `canManage`,
 * `callerRole`, and the store's `myWorkspaceRole` resolve consistently.
 */
function setup(callerRole: 'owner' | 'admin', extra: PrincipalDto[] = []) {
  const auth = useAuthStore();
  auth.user = { id: 'me', is_root: false, is_system_admin: false } as never;

  const workspace = useWorkspaceStore();
  workspace.activeWorkspaceSlug = 'acme';
  workspace.members = [userMember('me', 'Me', callerRole), ...extra];

  vi.spyOn(workspace, 'loadMembers').mockResolvedValue(undefined);

  return workspace;
}

let activeWrapper: VueWrapper | null = null;

function mountPanel(): VueWrapper {
  const wrapper = mount(MembersPanel, { attachTo: document.body });
  activeWrapper = wrapper;
  return wrapper;
}

afterEach(() => {
  activeWrapper?.unmount();
  activeWrapper = null;
  document.body.innerHTML = '';
});

async function openDialog(wrapper: VueWrapper): Promise<void> {
  await flushPromises();
  await wrapper.find('[data-action="add-member"]').trigger('click');
  await flushPromises();
}

describe('MembersPanel — sections', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('renders agents in their own section without role controls', async () => {
    const workspace = setup('owner', [agentMember('a1', 'CI Bot', 'agent')]);

    const wrapper = mountPanel();
    await flushPromises();

    const agentRows = wrapper.findAll('[data-agent-row]');
    expect(agentRows).toHaveLength(1);
    expect(agentRows[0]?.text()).toContain('CI Bot');
    expect(agentRows[0]?.text()).toContain('AGENT');

    expect(wrapper.find('[data-agent-row]').findComponent(Dropdown).exists()).toBe(false);
    expect(wrapper.find('[data-agent-row] .atl-member-remove').exists()).toBe(false);

    // The human member still keeps its role dropdown.
    expect(wrapper.find('[data-member-row]').findComponent(Dropdown).exists()).toBe(true);

    void workspace;
  });
});

describe('MembersPanel — add member dialog', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('opens the dialog and lists assignable users', async () => {
    const workspace = setup('owner');
    vi.spyOn(workspace, 'loadAssignableUsers').mockImplementation(async () => {
      workspace.assignableUsers = [
        assignable('u1', 'alice', 'Alice', true),
        assignable('u2', 'bob', 'Bob', false),
      ];
    });

    const wrapper = mountPanel();
    await wrapper.vm.$nextTick();

    await openDialog(wrapper);

    expect(workspace.loadAssignableUsers).toHaveBeenCalledWith('acme');

    const options = dialogAll('[data-add-user-option]');
    expect(options).toHaveLength(2);
    expect(options[0]?.textContent).toContain('Alice');
    expect(options[0]?.textContent).toContain('@alice');
    // Bob is not yet activated, so a pending badge shows.
    expect(options[1]?.textContent).toContain('Pending');
  });

  it('confirming calls addMember with the selected user and role, then reloads', async () => {
    const workspace = setup('owner');
    vi.spyOn(workspace, 'loadAssignableUsers').mockImplementation(async () => {
      workspace.assignableUsers = [assignable('u1', 'alice', 'Alice', true)];
    });
    const add = vi.spyOn(workspace, 'addMember').mockResolvedValue(true);

    const wrapper = mountPanel();
    await wrapper.vm.$nextTick();

    await openDialog(wrapper);

    dialogEl('[data-add-user-option]')?.dispatchEvent(new Event('click', { bubbles: true }));
    await wrapper.vm.$nextTick();

    await pickRole('Admin');

    dialogEl('[data-action="confirm-add"]')?.dispatchEvent(new Event('click', { bubbles: true }));
    await wrapper.vm.$nextTick();

    expect(add).toHaveBeenCalledWith('acme', 'u1', 'admin');
    expect(workspace.loadMembers).toHaveBeenCalledWith('acme');
  });

  it('hides the owner role option when the caller is an admin', async () => {
    const workspace = setup('admin');
    vi.spyOn(workspace, 'loadAssignableUsers').mockImplementation(async () => {
      workspace.assignableUsers = [assignable('u1', 'alice', 'Alice', true)];
    });

    const wrapper = mountPanel();
    await wrapper.vm.$nextTick();

    await openDialog(wrapper);

    await openRoleDropdown();

    const labels = roleOptionLabels();
    expect(labels).toContain('Admin');
    expect(labels).toContain('Member');
    expect(labels).not.toContain('Owner');
  });

  it('offers the owner role option when the caller is an owner', async () => {
    const workspace = setup('owner');
    vi.spyOn(workspace, 'loadAssignableUsers').mockImplementation(async () => {
      workspace.assignableUsers = [assignable('u1', 'alice', 'Alice', true)];
    });

    const wrapper = mountPanel();
    await wrapper.vm.$nextTick();

    await openDialog(wrapper);

    await openRoleDropdown();

    expect(roleOptionLabels()).toContain('Owner');
  });

  it('shows a friendly empty state when there are no assignable users', async () => {
    const workspace = setup('owner');
    vi.spyOn(workspace, 'loadAssignableUsers').mockImplementation(async () => {
      workspace.assignableUsers = [];
    });

    const wrapper = mountPanel();
    await wrapper.vm.$nextTick();

    await openDialog(wrapper);

    expect(dialogEl('[data-add-empty]')).not.toBeNull();
  });

  it('shows the server hint inline when addMember fails', async () => {
    const workspace = setup('owner');
    vi.spyOn(workspace, 'loadAssignableUsers').mockImplementation(async () => {
      workspace.assignableUsers = [assignable('u1', 'alice', 'Alice', true)];
    });
    workspace.error = 'User is already a member';
    vi.spyOn(workspace, 'addMember').mockResolvedValue(false);

    const wrapper = mountPanel();
    await wrapper.vm.$nextTick();

    await openDialog(wrapper);

    dialogEl('[data-add-user-option]')?.dispatchEvent(new Event('click', { bubbles: true }));
    await wrapper.vm.$nextTick();

    dialogEl('[data-action="confirm-add"]')?.dispatchEvent(new Event('click', { bubbles: true }));
    await wrapper.vm.$nextTick();
    await wrapper.vm.$nextTick();

    expect(dialogEl('[data-add-error]')?.textContent).toContain('User is already a member');
  });
});
