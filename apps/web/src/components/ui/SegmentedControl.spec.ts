import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import SegmentedControl from '@/components/ui/SegmentedControl.vue';

const THEME_OPTIONS = [
  { value: 'dark', label: 'Dark', icon: 'moon' },
  { value: 'light', label: 'Light', icon: 'sun' },
];

describe('SegmentedControl', () => {
  it('renders one option button per option, labelled in order', () => {
    const wrapper = mount(SegmentedControl, {
      props: { modelValue: 'dark', options: THEME_OPTIONS },
    });

    const options = wrapper.findAll('button.atl-seg-opt');

    expect(options).toHaveLength(2);
    expect(options.map((option) => option.text())).toEqual(['Dark', 'Light']);
  });

  it('marks only the option matching the model value as active', () => {
    const wrapper = mount(SegmentedControl, {
      props: { modelValue: 'light', options: THEME_OPTIONS },
    });

    const active = wrapper.findAll('button.atl-seg-opt.on');

    expect(active).toHaveLength(1);
    expect(active[0]?.text()).toBe('Light');
  });

  it('emits the option value when an inactive option is clicked', async () => {
    const wrapper = mount(SegmentedControl, {
      props: { modelValue: 'dark', options: THEME_OPTIONS },
    });

    await wrapper.findAll('button.atl-seg-opt')[1]?.trigger('click');

    expect(wrapper.emitted('update:modelValue')?.[0]).toEqual(['light']);
  });

  it('emits the option value when the active option is clicked', async () => {
    const wrapper = mount(SegmentedControl, {
      props: { modelValue: 'dark', options: THEME_OPTIONS },
    });

    await wrapper.findAll('button.atl-seg-opt')[0]?.trigger('click');

    expect(wrapper.emitted('update:modelValue')?.[0]).toEqual(['dark']);
  });

  it('renders options without an icon', () => {
    const wrapper = mount(SegmentedControl, {
      props: {
        modelValue: 'on',
        options: [
          { value: 'on', label: 'On' },
          { value: 'off', label: 'Off' },
        ],
      },
    });

    expect(wrapper.findAll('button.atl-seg-opt').map((option) => option.text())).toEqual(['On', 'Off']);
  });
});
