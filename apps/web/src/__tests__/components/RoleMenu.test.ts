import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import RoleMenu from '@/components/share/RoleMenu.vue';

describe('RoleMenu (REQ-W27 — agents capped at editor, E03 guard)', () => {
  it('offers Viewer, Editor and Admin for a user principal', () => {
    const wrapper = mount(RoleMenu, { props: { principalType: 'user', role: 'editor' } });

    const items = wrapper.findAll('[data-role-option]');
    const values = items.map((i) => i.attributes('data-role-option'));

    expect(values).toContain('viewer');
    expect(values).toContain('editor');
    expect(values).toContain('admin');

    const admin = wrapper.find('[data-role-option="admin"]');
    expect(admin.attributes('aria-disabled')).not.toBe('true');
  });

  it('does NOT offer a selectable Admin for an api_key agent', () => {
    const wrapper = mount(RoleMenu, { props: { principalType: 'api_key', role: 'editor' } });

    const admin = wrapper.find('[data-role-option="admin"]');

    if (admin.exists()) {
      expect(admin.attributes('aria-disabled')).toBe('true');
    }
    expect(wrapper.find('[data-selectable-role="admin"]').exists()).toBe(false);
  });

  it('omits the Admin option entirely for an api_key agent but offers it for a group', () => {
    const agent = mount(RoleMenu, { props: { principalType: 'api_key', role: 'editor' } });
    expect(agent.find('[data-role-option="admin"]').exists()).toBe(false);
    expect(agent.find('[data-role-option="viewer"]').exists()).toBe(true);
    expect(agent.find('[data-role-option="editor"]').exists()).toBe(true);

    const group = mount(RoleMenu, { props: { principalType: 'group', role: 'editor' } });
    expect(group.find('[data-role-option="admin"]').exists()).toBe(true);
  });

  it('does not emit a select event when an agent Admin option is clicked', async () => {
    const wrapper = mount(RoleMenu, { props: { principalType: 'api_key', role: 'editor' } });

    const admin = wrapper.find('[data-role-option="admin"]');
    if (admin.exists()) {
      await admin.trigger('click');
    }

    const emitted = wrapper.emitted('select');
    const adminEmitted = (emitted ?? []).some((args) => args[0] === 'admin');
    expect(adminEmitted).toBe(false);
  });

  it('emits select with the chosen role for an allowed option', async () => {
    const wrapper = mount(RoleMenu, { props: { principalType: 'user', role: 'editor' } });

    await wrapper.find('[data-selectable-role="viewer"]').trigger('click');

    expect(wrapper.emitted('select')?.[0]).toEqual(['viewer']);
  });

  it('shows the agent cap note for an api_key principal', () => {
    const wrapper = mount(RoleMenu, { props: { principalType: 'api_key', role: 'editor' } });
    expect(wrapper.text().toLowerCase()).toContain('editor max');
  });

  it('emits remove when Remove is clicked', async () => {
    const wrapper = mount(RoleMenu, { props: { principalType: 'user', role: 'editor' } });

    await wrapper.find('[data-action="remove"]').trigger('click');

    expect(wrapper.emitted('remove')).toBeTruthy();
  });
});
