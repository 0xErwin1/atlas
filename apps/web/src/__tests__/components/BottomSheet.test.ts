import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import { h } from 'vue';
import BottomSheet from '@/components/ui/BottomSheet.vue';

describe('BottomSheet', () => {
  it('renders its slotted content when open', () => {
    const wrapper = mount(BottomSheet, {
      props: { open: true },
      slots: { default: () => h('p', 'sheet body') },
    });

    expect(wrapper.text()).toContain('sheet body');
  });

  it('renders nothing when closed', () => {
    const wrapper = mount(BottomSheet, {
      props: { open: false },
      slots: { default: () => h('p', 'sheet body') },
    });

    expect(wrapper.text()).not.toContain('sheet body');
    expect(wrapper.find('[role="dialog"]').exists()).toBe(false);
  });

  it('shows the title when provided', () => {
    const wrapper = mount(BottomSheet, {
      props: { open: true, title: 'Details' },
    });

    expect(wrapper.text()).toContain('Details');
  });

  it('emits close when the backdrop is clicked', async () => {
    const wrapper = mount(BottomSheet, { props: { open: true } });

    await wrapper.find('[data-sheet-backdrop]').trigger('click');

    expect(wrapper.emitted('close')).toHaveLength(1);
  });

  it('emits close when the close button is clicked', async () => {
    const wrapper = mount(BottomSheet, { props: { open: true, title: 'Details' } });

    await wrapper.find('[data-action="close"]').trigger('click');

    expect(wrapper.emitted('close')).toHaveLength(1);
  });

  it('emits close on Escape', async () => {
    const wrapper = mount(BottomSheet, { props: { open: true }, attachTo: document.body });

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    await wrapper.vm.$nextTick();

    expect(wrapper.emitted('close')).toHaveLength(1);
    wrapper.unmount();
  });
});
