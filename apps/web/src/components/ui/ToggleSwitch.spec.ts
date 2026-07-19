import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import ToggleSwitch from '@/components/ui/ToggleSwitch.vue';

describe('ToggleSwitch', () => {
  it('renders the label and copy and reflects the on state', () => {
    const wrapper = mount(ToggleSwitch, {
      props: { modelValue: true, label: 'Global agent', copy: 'Reaches everything.' },
    });

    expect(wrapper.text()).toContain('Global agent');
    expect(wrapper.text()).toContain('Reaches everything.');

    const button = wrapper.get('[data-switch]');
    expect(button.attributes('role')).toBe('switch');
    expect(button.attributes('aria-checked')).toBe('true');
  });

  it('emits the negated value on click', async () => {
    const wrapper = mount(ToggleSwitch, { props: { modelValue: false, label: 'On' } });

    await wrapper.get('[data-switch]').trigger('click');

    expect(wrapper.emitted('update:modelValue')?.at(-1)).toEqual([true]);
  });

  it('does not emit while disabled', async () => {
    const wrapper = mount(ToggleSwitch, {
      props: { modelValue: false, label: 'On', disabled: true },
    });

    await wrapper.get('[data-switch]').trigger('click');

    expect(wrapper.emitted('update:modelValue')).toBeUndefined();
  });

  it('forwards attributes such as data-action and aria-label onto the switch', () => {
    const wrapper = mount(ToggleSwitch, {
      props: { modelValue: false, label: 'On' },
      attrs: { 'data-action': 'toggle-sysadmin', 'aria-label': 'System admin' },
    });

    const button = wrapper.get('[data-switch]');
    expect(button.attributes('data-action')).toBe('toggle-sysadmin');
    expect(button.attributes('aria-label')).toBe('System admin');
  });
});
