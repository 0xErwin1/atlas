import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import ScopeGrid from '@/components/settings/ScopeGrid.vue';

describe('ScopeGrid', () => {
  it('forwards each scope token and gives each checkbox a concrete accessible name', () => {
    const wrapper = mount(ScopeGrid, { props: { modelValue: ['tasks:read'] } });
    const checkbox = wrapper.get<HTMLInputElement>('input[data-scope="tasks:read"]');

    expect(checkbox.attributes('aria-label')).toBe('tasks: read capability');
    expect(checkbox.element.checked).toBe(true);
  });

  it('disables every capability checkbox when the grid is disabled', () => {
    const wrapper = mount(ScopeGrid, { props: { modelValue: [], disabled: true } });
    const checkboxes = wrapper.findAll('input[type="checkbox"]');

    expect(checkboxes.length).toBeGreaterThan(0);
    expect(checkboxes.every((checkbox) => checkbox.attributes('disabled') !== undefined)).toBe(true);
  });

  it('emits the canonical selection when a capability is requested', async () => {
    const wrapper = mount(ScopeGrid, { props: { modelValue: [] } });

    await wrapper.get('input[data-scope="tasks:create"]').setValue(true);

    expect(wrapper.emitted('update:modelValue')).toEqual([[['tasks:create']]]);
  });
});
