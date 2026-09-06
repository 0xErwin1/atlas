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

  it('calls only /api/v2/platform/meta, never /version and never /api/meta', async () => {
    GET.mockResolvedValueOnce({ data: { version: '1.2.3', build: 'abc', url: null } });

    mount(AboutPanel);
    await flushPromises();

    expect(GET).toHaveBeenCalledTimes(1);
    expect(GET).toHaveBeenCalledWith('/api/v2/platform/meta', {});
    expect(GET).not.toHaveBeenCalledWith('/version', expect.anything());
    expect(GET).not.toHaveBeenCalledWith('/api/meta', expect.anything());
  });

  it('renders only version, build, and url from ServerMetaDto, ignoring retired fields', async () => {
    GET.mockResolvedValueOnce({
      data: {
        version: '1.2.3',
        build: 'abc',
        url: 'https://atlas.internal',
        max_attachment_bytes: 999,
        semantic_search_enabled: true,
      },
    });

    const wrapper = mount(AboutPanel);
    await flushPromises();

    expect(wrapper.text()).toContain('1.2.3');
    expect(wrapper.text()).toContain('abc');
    expect(wrapper.text()).toContain('https://atlas.internal');
    expect(wrapper.text()).not.toContain('999');
    expect(wrapper.text()).not.toContain('max_attachment_bytes');
    expect(wrapper.text()).not.toContain('semantic_search_enabled');
  });
});
