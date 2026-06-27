import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import SettingsTable from '@/components/settings/SettingsTable.vue';

describe('SettingsTable', () => {
  it('renders the head slot inside the header row', () => {
    const wrapper = mount(SettingsTable, {
      slots: {
        head: '<div class="col">Name</div>',
        default: '<div class="row">a row</div>',
      },
    });

    const head = wrapper.find('.atl-settings-head');
    expect(head.exists()).toBe(true);
    expect(head.find('.col').text()).toBe('Name');
  });

  it('renders default-slot rows inside the bordered container', () => {
    const wrapper = mount(SettingsTable, {
      slots: {
        head: '<div>head</div>',
        default: '<div class="row">first</div><div class="row">second</div>',
      },
    });

    const container = wrapper.find('.atl-settings-table');
    expect(container.exists()).toBe(true);
    expect(container.findAll('.row')).toHaveLength(2);
  });
});
