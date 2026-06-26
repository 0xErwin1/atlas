import { mount, type VueWrapper } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import WorkspaceAccessEditor, {
  type RoleOption,
  type WorkspaceRef,
} from '@/components/settings/WorkspaceAccessEditor.vue';

// ConfirmDialog teleports to <body>, so its nodes live outside the wrapper.
function dialogEl<T extends Element = HTMLElement>(selector: string): T | null {
  return document.body.querySelector<T>(selector);
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

  it('renders one row per workspace and selects None for unassigned ones', () => {
    const wrapper = mountEditor({});

    const rows = wrapper.findAll('[data-wsa-row]');
    expect(rows).toHaveLength(2);

    const selects = wrapper.findAll<HTMLSelectElement>('[data-wsa-role]');
    expect(selects[0]?.element.value).toBe('');
    expect(selects[1]?.element.value).toBe('');
  });

  it('emits assign(slug, role) when a role is selected', async () => {
    const wrapper = mountEditor({});

    const select = wrapper.find<HTMLSelectElement>('[data-wsa-role]');
    select.element.value = 'admin';
    await select.trigger('change');

    expect(wrapper.emitted('assign')).toEqual([['acme', 'admin']]);
    expect(wrapper.emitted('remove')).toBeUndefined();
  });

  it('confirms before emitting remove(slug) when an existing role is set to None', async () => {
    const wrapper = mountEditor({ acme: 'admin' });

    const select = wrapper.find<HTMLSelectElement>('[data-wsa-role]');
    expect(select.element.value).toBe('admin');

    select.element.value = '';
    await select.trigger('change');

    // No remove until the confirmation is accepted.
    expect(wrapper.emitted('remove')).toBeUndefined();

    dialogEl('[data-test="confirm"]')?.dispatchEvent(new Event('click', { bubbles: true }));
    await wrapper.vm.$nextTick();

    expect(wrapper.emitted('remove')).toEqual([['acme']]);
  });

  it('does not emit remove when the confirmation is cancelled', async () => {
    const wrapper = mountEditor({ acme: 'admin' });

    const select = wrapper.find<HTMLSelectElement>('[data-wsa-role]');
    select.element.value = '';
    await select.trigger('change');

    dialogEl('[data-test="cancel"]')?.dispatchEvent(new Event('click', { bubbles: true }));
    await wrapper.vm.$nextTick();

    expect(wrapper.emitted('remove')).toBeUndefined();
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
