import { type DOMWrapper, mount, type VueWrapper } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import WorkspaceAccessEditor, {
  type RoleOption,
  type WorkspaceRef,
} from '@/components/settings/WorkspaceAccessEditor.vue';

// ConfirmDialog teleports to <body>, so its nodes live outside the wrapper.
function dialogEl<T extends Element = HTMLElement>(selector: string): T | null {
  return document.body.querySelector<T>(selector);
}

// The role control is now a Dropdown: click its trigger button to open the
// listbox, then click the option whose label matches.
async function pickRole(row: DOMWrapper<Element>, label: string): Promise<void> {
  await row.find('button').trigger('click');
  const option = row.findAll('li[role="option"]').find((li) => li.text() === label);
  if (option === undefined) throw new Error(`option not found: ${label}`);
  await option.trigger('click');
}

function roleTrigger(row: DOMWrapper<Element>): string {
  return row.find('button').text();
}

function rowAt(wrapper: VueWrapper, index: number): DOMWrapper<Element> {
  const row = wrapper.findAll('[data-wsa-row]')[index];
  if (row === undefined) throw new Error(`workspace row not found: ${index}`);
  return row;
}

const WORKSPACES: WorkspaceRef[] = [
  { slug: 'acme', name: 'Acme' },
  { slug: 'beta', name: 'Beta' },
];

const OPTIONS: RoleOption[] = [
  { value: 'member', label: 'Member' },
  { value: 'admin', label: 'Admin' },
];

let activeWrapper: VueWrapper | null = null;

function mountEditor(roles: Record<string, string>): VueWrapper {
  const wrapper = mount(WorkspaceAccessEditor, {
    attachTo: document.body,
    props: { workspaces: WORKSPACES, roles, options: OPTIONS },
  });
  activeWrapper = wrapper;
  return wrapper;
}

afterEach(() => {
  activeWrapper?.unmount();
  activeWrapper = null;
  document.body.innerHTML = '';
});

describe('WorkspaceAccessEditor', () => {
  beforeEach(() => {
    document.body.innerHTML = '';
  });

  it('renders one row per workspace and shows None for unassigned ones', () => {
    const wrapper = mountEditor({});

    const rows = wrapper.findAll('[data-wsa-row]');
    expect(rows).toHaveLength(2);

    expect(roleTrigger(rowAt(wrapper, 0))).toContain('None');
    expect(roleTrigger(rowAt(wrapper, 1))).toContain('None');
  });

  it('emits assign(slug, role) when a role is selected', async () => {
    const wrapper = mountEditor({});

    await pickRole(rowAt(wrapper, 0), 'Admin');

    expect(wrapper.emitted('assign')).toEqual([['acme', 'admin']]);
    expect(wrapper.emitted('remove')).toBeUndefined();
  });

  it('confirms before emitting remove(slug) when an existing role is set to None', async () => {
    const wrapper = mountEditor({ acme: 'admin' });

    const row = rowAt(wrapper, 0);
    expect(roleTrigger(row)).toContain('Admin');

    await pickRole(row, 'None');

    // No remove until the confirmation is accepted.
    expect(wrapper.emitted('remove')).toBeUndefined();

    dialogEl('[data-test="confirm"]')?.dispatchEvent(new Event('click', { bubbles: true }));
    await wrapper.vm.$nextTick();

    expect(wrapper.emitted('remove')).toEqual([['acme']]);
  });

  it('does not emit remove when the confirmation is cancelled and keeps the role shown', async () => {
    const wrapper = mountEditor({ acme: 'admin' });

    const row = rowAt(wrapper, 0);
    await pickRole(row, 'None');

    dialogEl('[data-test="cancel"]')?.dispatchEvent(new Event('click', { bubbles: true }));
    await wrapper.vm.$nextTick();

    expect(wrapper.emitted('remove')).toBeUndefined();
    // The displayed value is driven by the prop, so it never changed optimistically.
    expect(roleTrigger(row)).toContain('Admin');
  });

  it('shows an empty state when there are no workspaces', () => {
    const wrapper = mount(WorkspaceAccessEditor, {
      props: { workspaces: [], roles: {}, options: OPTIONS },
    });
    activeWrapper = wrapper;

    expect(wrapper.find('[data-wsa-row]').exists()).toBe(false);
    expect(wrapper.text()).toContain('No workspaces to assign yet.');
  });
});
