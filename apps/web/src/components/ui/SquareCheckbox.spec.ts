import { mount } from '@vue/test-utils';
import { describe, expect, it, vi } from 'vitest';
import SquareCheckbox from '@/components/ui/SquareCheckbox.vue';

describe('SquareCheckbox', () => {
  it.each([
    { modelValue: false, checked: false },
    { modelValue: true, checked: true },
  ])('renders the controlled checked state', ({ modelValue, checked }) => {
    const wrapper = mount(SquareCheckbox, { props: { modelValue, label: 'Example' } });

    expect(wrapper.get<HTMLInputElement>('input').element.checked).toBe(checked);
  });

  it('emits the requested next value', async () => {
    const wrapper = mount(SquareCheckbox, { props: { modelValue: false, label: 'Example' } });

    await wrapper.get('input').setValue(true);

    expect(wrapper.emitted('update:modelValue')).toEqual([[true]]);
  });

  it('restores the controlled DOM state until the parent updates the prop', async () => {
    const wrapper = mount(SquareCheckbox, { props: { modelValue: false, label: 'Example' } });
    const input = wrapper.get<HTMLInputElement>('input');

    await input.setValue(true);

    expect(input.element.checked).toBe(false);

    await wrapper.setProps({ modelValue: true });

    expect(input.element.checked).toBe(true);
  });

  it('does not emit while disabled', async () => {
    const wrapper = mount(SquareCheckbox, {
      props: { modelValue: false, label: 'Example', disabled: true },
    });

    await wrapper.get('input').setValue(true);

    expect(wrapper.get('input').attributes('disabled')).toBeDefined();
    expect(wrapper.emitted('update:modelValue')).toBeUndefined();
  });

  it('provides its accessible label and forwards input attributes and listeners', async () => {
    const onFocus = vi.fn();
    const wrapper = mount(SquareCheckbox, {
      props: { modelValue: false, label: 'Allow task updates' },
      attrs: {
        id: 'task-updates',
        name: 'capability',
        'data-scope': 'tasks:update',
        onFocus,
      },
    });

    const input = wrapper.get('input');
    await input.trigger('focus');

    expect(input.attributes('aria-label')).toBe('Allow task updates');
    expect(input.attributes('id')).toBe('task-updates');
    expect(input.attributes('name')).toBe('capability');
    expect(input.attributes('data-scope')).toBe('tasks:update');
    expect(onFocus).toHaveBeenCalledTimes(1);
  });

  it.each(['primary', 'success'] as const)('exposes the %s tone visual hook', (tone) => {
    const wrapper = mount(SquareCheckbox, { props: { modelValue: true, label: 'Example', tone } });

    expect(wrapper.get('input').classes()).toContain(`atl-square-checkbox--${tone}`);
  });
});
