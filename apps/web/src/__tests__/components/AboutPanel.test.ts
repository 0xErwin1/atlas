import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET } = vi.hoisted(() => ({ GET: vi.fn() }));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET },
}));

import AboutPanel from '@/components/settings/AboutPanel.vue';

describe('AboutPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders the URL row when the server reports a url', async () => {
    GET.mockResolvedValueOnce({ data: { version: '1.2.3', build: 'abc', url: 'https://atlas.internal' } });

    const wrapper = mount(AboutPanel);
    await flushPromises();

    expect(wrapper.text()).toContain('URL');
    expect(wrapper.text()).toContain('https://atlas.internal');
  });

  it('omits the URL row when no url is reported', async () => {
    GET.mockResolvedValueOnce({ data: { version: '1.2.3', build: null } });

    const wrapper = mount(AboutPanel);
    await flushPromises();

    expect(wrapper.text()).not.toContain('URL');
    expect(wrapper.text()).toContain('1.2.3');
  });
});
