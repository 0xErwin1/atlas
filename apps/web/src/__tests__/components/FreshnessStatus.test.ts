import { mount } from '@vue/test-utils';
import { describe, expect, it } from 'vitest';
import FreshnessStatus from '@/components/states/FreshnessStatus.vue';

describe('FreshnessStatus', () => {
  it('shows a non-blocking degraded status and preserves retry affordance', async () => {
    const wrapper = mount(FreshnessStatus, { props: { status: 'error-with-data' } });

    expect(wrapper.text()).toContain('Showing saved data');
    await wrapper.get('button').trigger('click');

    expect(wrapper.emitted('retry')).toHaveLength(1);
  });

  it('does not render a status surface when data is ready', () => {
    const wrapper = mount(FreshnessStatus, { props: { status: 'ready' } });

    expect(wrapper.find('[role="status"]').exists()).toBe(false);
  });
});
