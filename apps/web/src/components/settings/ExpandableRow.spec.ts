import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import ExpandableRow from '@/components/settings/ExpandableRow.vue';
import Icon from '@/components/ui/Icon.vue';

function mountRow(props: { expanded?: boolean; expandable?: boolean } = {}) {
  return mount(ExpandableRow, {
    props: { expanded: false, ...props },
    slots: {
      summary: '<div class="summary-cell">summary</div>',
      actions: '<button class="act-btn" type="button">Act</button>',
      panel: '<div class="panel-body">panel</div>',
    },
  });
}

describe('ExpandableRow', () => {
  it('emits toggle when the row is clicked', async () => {
    const wrapper = mountRow();

    await wrapper.find('[data-row]').trigger('click');

    expect(wrapper.emitted('toggle')).toHaveLength(1);
  });

  it('emits toggle when the Manage button is clicked', async () => {
    const wrapper = mountRow();

    const manage = wrapper.find('[data-action="manage"]');
    expect(manage.exists()).toBe(true);
    expect(manage.text()).toContain('Manage');

    await manage.trigger('click');

    expect(wrapper.emitted('toggle')).toHaveLength(1);
  });

  it('does not toggle when an action-slot button is clicked', async () => {
    const wrapper = mountRow();

    await wrapper.find('.act-btn').trigger('click');

    expect(wrapper.emitted('toggle')).toBeUndefined();
  });

  it('renders the panel slot only when expanded', async () => {
    const collapsed = mountRow({ expanded: false });
    expect(collapsed.find('.panel-body').exists()).toBe(false);
    expect(collapsed.find('[data-row-panel]').exists()).toBe(false);

    const expanded = mountRow({ expanded: true });
    expect(expanded.find('.panel-body').exists()).toBe(true);
    expect(expanded.find('[data-row-panel]').exists()).toBe(true);
  });

  it('renders no Manage button and is not clickable when expandable is false', async () => {
    const wrapper = mountRow({ expandable: false });

    expect(wrapper.find('[data-action="manage"]').exists()).toBe(false);

    await wrapper.find('[data-row]').trigger('click');
    expect(wrapper.emitted('toggle')).toBeUndefined();

    expect(wrapper.find('[data-row]').classes()).not.toContain('atl-erow--clickable');
  });

  it('chevron flips with the expanded state', () => {
    const collapsed = mountRow({ expanded: false });
    const collapsedIcons = collapsed.findAllComponents(Icon).map((c) => c.props('name'));
    expect(collapsedIcons).toContain('chevron-down');
    expect(collapsedIcons).not.toContain('chevron-up');

    const expanded = mountRow({ expanded: true });
    const expandedIcons = expanded.findAllComponents(Icon).map((c) => c.props('name'));
    expect(expandedIcons).toContain('chevron-up');
  });
});
