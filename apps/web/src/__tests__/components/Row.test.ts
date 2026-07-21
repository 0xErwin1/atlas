import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import Row from '@/components/ui/Row.vue';

describe('Row', () => {
  it('keeps the default 14px indentation step', () => {
    const wrapper = mount(Row, { props: { label: 'Nested', depth: 1 } });

    expect(wrapper.get('.atl-row').attributes('style')).toContain('padding-left: 22px');
  });

  it('supports a local indentation step override', () => {
    const wrapper = mount(Row, { props: { label: 'Tree child', depth: 1, depthStep: 20 } });

    expect(wrapper.get('.atl-row').attributes('style')).toContain('padding-left: 28px');
  });
});
