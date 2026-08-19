import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import Row from '@/components/ui/Row.vue';

describe('Row', () => {
  it('exposes the complete navigation label through its native title', () => {
    const wrapper = mount(Row, { props: { label: 'A deeply nested project folder name' } });

    expect(wrapper.get('.truncate').attributes('title')).toBe('A deeply nested project folder name');
  });

  // Blueprint indents the tree on the 8/24/40 rail: the root sits at 8 and each
  // level adds one 16px step, all of them on the spacing scale.
  it.each([
    [0, '8px'],
    [1, '24px'],
    [2, '40px'],
  ])('indents depth %i to %s by default', (depth, padding) => {
    const wrapper = mount(Row, { props: { label: 'Nested', depth } });

    expect(wrapper.get('.atl-row').attributes('style')).toContain(`padding-left: ${padding}`);
  });

  it('sits on the shared navigation row height', () => {
    const wrapper = mount(Row, { props: { label: 'Nested' } });

    expect(wrapper.get('.atl-row').attributes('style')).toContain('height: var(--h-row)');
  });

  it('supports a local indentation step override', () => {
    const wrapper = mount(Row, { props: { label: 'Tree child', depth: 1, depthStep: 20 } });

    expect(wrapper.get('.atl-row').attributes('style')).toContain('padding-left: 28px');
  });
});
