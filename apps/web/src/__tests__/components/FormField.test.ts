import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import FormField from '@/components/ui/FormField.vue';

describe('FormField', () => {
  it('emits update:modelValue on input', async () => {
    const wrapper = mount(FormField, { props: { modelValue: '', label: 'Email' } });

    await wrapper.get('input').setValue('hi');

    expect(wrapper.emitted('update:modelValue')?.[0]).toEqual(['hi']);
  });

  it('shows the inline error and marks the input invalid', () => {
    const wrapper = mount(FormField, {
      props: { modelValue: '', label: 'Email', error: 'Email is required' },
    });

    expect(wrapper.text()).toContain('Email is required');
    expect(wrapper.get('input').attributes('aria-invalid')).toBe('true');
  });

  it('renders helper text only when there is no error', async () => {
    const wrapper = mount(FormField, {
      props: { modelValue: '', helper: 'Optional', error: null },
    });
    expect(wrapper.text()).toContain('Optional');

    await wrapper.setProps({ error: 'Required' });
    expect(wrapper.text()).toContain('Required');
    expect(wrapper.text()).not.toContain('Optional');
  });

  it('toggles password visibility', async () => {
    const wrapper = mount(FormField, {
      props: { modelValue: 'secret', type: 'password' },
    });

    expect(wrapper.get('input').attributes('type')).toBe('password');

    await wrapper.get('button').trigger('click');

    expect(wrapper.get('input').attributes('type')).toBe('text');
  });
});
