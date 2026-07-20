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

  it('suppresses the routine refreshing row when asked, without hiding genuine signals', () => {
    const refreshing = mount(FreshnessStatus, {
      props: { status: 'refreshing', suppressRefreshing: true },
    });
    expect(refreshing.find('[role="status"]').exists()).toBe(false);

    const offline = mount(FreshnessStatus, {
      props: { status: 'offline', suppressRefreshing: true },
    });
    expect(offline.text()).toContain('Offline');
  });

  it('still shows the refreshing row by default', () => {
    const wrapper = mount(FreshnessStatus, { props: { status: 'refreshing' } });

    expect(wrapper.text()).toContain('Updating…');
  });
});
