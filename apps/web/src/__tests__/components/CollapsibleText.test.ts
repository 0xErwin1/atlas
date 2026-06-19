import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import CollapsibleText from '@/components/ui/CollapsibleText.vue';

interface Measurable {
  measure: () => void;
}

function mountWith(scrollHeight: number) {
  const wrapper = mount(CollapsibleText, {
    props: { collapsedHeight: 100 },
    slots: { default: '<p>content</p>' },
  });

  const content = wrapper.find('.atl-collapsible-content').element;
  Object.defineProperty(content, 'scrollHeight', { configurable: true, value: scrollHeight });
  (wrapper.vm as unknown as Measurable).measure();

  return wrapper;
}

describe('CollapsibleText', () => {
  it('shows no toggle and does not clamp when the content fits', async () => {
    const wrapper = mountWith(50);
    await wrapper.vm.$nextTick();

    expect(wrapper.find('.atl-collapsible-toggle').exists()).toBe(false);
    expect(wrapper.find('.atl-collapsible-content').classes()).not.toContain('clamped');
  });

  it('clamps overflowing content and expands on toggle', async () => {
    const wrapper = mountWith(400);
    await wrapper.vm.$nextTick();

    const toggle = wrapper.find('.atl-collapsible-toggle');
    expect(toggle.exists()).toBe(true);
    expect(wrapper.find('.atl-collapsible-content').classes()).toContain('clamped');

    await toggle.trigger('click');

    expect(wrapper.attributes('data-expanded')).toBe('true');
    expect(wrapper.find('.atl-collapsible-content').classes()).not.toContain('clamped');
  });
});
