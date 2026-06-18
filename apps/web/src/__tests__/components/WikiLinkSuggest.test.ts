import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET } = vi.hoisted(() => ({ GET: vi.fn() }));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET },
}));

import WikiLinkSuggest from '@/components/notas/WikiLinkSuggest.vue';

const hit = (id: string, title: string) => ({
  id,
  title,
  kind: 'note',
  score: 1,
  updated_at: '2026-01-01T00:00:00Z',
});

describe('WikiLinkSuggest', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('is hidden when there is no active query', () => {
    GET.mockResolvedValue({ data: { items: [], has_more: false } });
    const wrapper = mount(WikiLinkSuggest, { props: { ws: 'ws', query: null } });
    expect(wrapper.find('[role="listbox"]').exists()).toBe(false);
  });

  it('fetches and lists note hits for an active query (REQ-W16)', async () => {
    GET.mockResolvedValue({ data: { items: [hit('d1', 'Architecture')], has_more: false } });

    const wrapper = mount(WikiLinkSuggest, { props: { ws: 'ws', query: 'arch' } });
    await flushPromises();

    expect(GET).toHaveBeenCalledOnce();
    expect(wrapper.text()).toContain('Architecture');
  });

  it('emits select with the hit title on click', async () => {
    GET.mockResolvedValue({ data: { items: [hit('d1', 'Architecture')], has_more: false } });

    const wrapper = mount(WikiLinkSuggest, { props: { ws: 'ws', query: 'arch' } });
    await flushPromises();

    const option = wrapper.findAll('[role="option"]').find((o) => o.text().includes('Architecture'));
    await option?.trigger('mousedown');

    expect(wrapper.emitted('select')?.[0]).toEqual([{ id: 'd1', title: 'Architecture' }]);
  });

  it('degrades gracefully on a network error and still offers creation (REQ-W16)', async () => {
    GET.mockResolvedValue({ error: { status: 500 } });

    const wrapper = mount(WikiLinkSuggest, { props: { ws: 'ws', query: 'newnote' } });
    await flushPromises();

    expect(wrapper.text()).toContain('Search unavailable');
    expect(wrapper.text()).toContain('Create');

    const createOption = wrapper.findAll('[role="option"]').find((o) => o.text().includes('Create'));
    await createOption?.trigger('mousedown');
    expect(wrapper.emitted('select')?.[0]).toEqual([{ id: null, title: 'newnote' }]);
  });

  it('confirms the active item via keyboard navigation', async () => {
    GET.mockResolvedValue({
      data: { items: [hit('d1', 'Alpha'), hit('d2', 'Beta')], has_more: false },
    });

    const wrapper = mount(WikiLinkSuggest, { props: { ws: 'ws', query: 'a' } });
    await flushPromises();

    wrapper.vm.moveDown();
    wrapper.vm.confirmActive();

    expect(wrapper.emitted('select')?.[0]).toEqual([{ id: 'd2', title: 'Beta' }]);
  });
});
