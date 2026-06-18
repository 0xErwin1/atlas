import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import PropertiesPanel from '@/components/notas/PropertiesPanel.vue';
import Avatar from '@/components/ui/Avatar.vue';
import Chip from '@/components/ui/Chip.vue';

describe('PropertiesPanel', () => {
  it('shows an empty message with no frontmatter', () => {
    const wrapper = mount(PropertiesPanel, { props: { meta: {} } });
    expect(wrapper.text()).toContain('No properties.');
  });

  it('renders status and each tag as chips', () => {
    const wrapper = mount(PropertiesPanel, {
      props: { meta: { status: 'In progress', tags: ['shell', 'M1'] } },
    });
    const chips = wrapper.findAllComponents(Chip).map((c) => c.text());
    expect(chips).toContain('In progress');
    expect(chips).toContain('shell');
    expect(chips).toContain('M1');
  });

  it('renders a person property with an avatar', () => {
    const wrapper = mount(PropertiesPanel, { props: { meta: { owner: 'Mara K' } } });
    expect(wrapper.findComponent(Avatar).exists()).toBe(true);
    expect(wrapper.text()).toContain('Mara K');
  });

  it('renders an unknown property as plain text, not a chip', () => {
    const wrapper = mount(PropertiesPanel, { props: { meta: { summary: 'just text' } } });
    expect(wrapper.findComponent(Chip).exists()).toBe(false);
    expect(wrapper.text()).toContain('just text');
  });
});
