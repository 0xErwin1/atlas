import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import AgentBadge from '@/components/ui/AgentBadge.vue';
import Avatar from '@/components/ui/Avatar.vue';

describe('AgentBadge', () => {
  it('reads AGENT by default with no glyph', () => {
    const wrapper = mount(AgentBadge);
    expect(wrapper.text()).toBe('AGENT');
    expect(wrapper.text()).not.toContain('✦');
  });

  it('honors a custom label', () => {
    const wrapper = mount(AgentBadge, { props: { label: 'SCRIPT' } });
    expect(wrapper.text()).toBe('SCRIPT');
  });
});

describe('Avatar', () => {
  it('shows the sparkles glyph for agents instead of initials', () => {
    const wrapper = mount(Avatar, { props: { agent: true, name: 'Claude', size: 18 } });
    expect(wrapper.findComponent({ name: 'Icon' }).exists()).toBe(true);
    expect(wrapper.text()).not.toContain('CL');
  });

  it('shows initials for non-agent users', () => {
    const wrapper = mount(Avatar, { props: { name: 'Mara K', size: 24 } });
    expect(wrapper.findComponent({ name: 'Icon' }).exists()).toBe(false);
    expect(wrapper.text()).toBe('MA');
  });
});
