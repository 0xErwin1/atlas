import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import Row from '@/components/ui/Row.vue';

describe('Row', () => {
  it('exposes the complete navigation label through its native title', () => {
    const wrapper = mount(Row, { props: { label: 'A deeply nested project folder name' } });

    expect(wrapper.get('.truncate').attributes('title')).toBe('A deeply nested project folder name');
  });

  it('keeps the default 14px indentation step', () => {
    const wrapper = mount(Row, { props: { label: 'Nested', depth: 1 } });

    expect(wrapper.get('.atl-row').attributes('style')).toContain('padding-left: 22px');
  });

  it('supports a local indentation step override', () => {
    const wrapper = mount(Row, { props: { label: 'Tree child', depth: 1, depthStep: 20 } });

    expect(wrapper.get('.atl-row').attributes('style')).toContain('padding-left: 28px');
  });
});
