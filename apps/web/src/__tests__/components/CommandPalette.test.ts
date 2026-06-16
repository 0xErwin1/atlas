import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const { GET } = vi.hoisted(() => ({
  GET: vi.fn(),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET },
}));

import CommandPalette from '@/components/search/CommandPalette.vue';
import { useSearchStore } from '@/stores/search';

const localActions = [
  { id: 'goto-notes', label: 'Go to Notes', kind: 'navigate' as const },
  { id: 'goto-tasks', label: 'Go to Tasks', kind: 'navigate' as const },
];

const mountPalette = () =>
  mount(CommandPalette, {
    props: { ws: 'acme', open: true, actions: localActions },
  });

describe('CommandPalette (REQ-W23/W24)', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    vi.useFakeTimers();
    GET.mockResolvedValue({
      data: { items: [], next_cursor: null, has_more: false },
      error: undefined,
    });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('renders local actions when there is no query', () => {
    const wrapper = mountPalette();
    expect(wrapper.text()).toContain('Go to Notes');
    expect(wrapper.text()).toContain('Go to Tasks');
  });

  it('queries the search API (debounced) and shows ranked hits', async () => {
    GET.mockResolvedValueOnce({
      data: {
        items: [
          { id: 'd1', kind: 'document', title: 'Shell doc', score: 2, updated_at: '2026-01-01T00:00:00Z' },
        ],
        next_cursor: null,
        has_more: false,
      },
      error: undefined,
    });

    const wrapper = mountPalette();
    await wrapper.get('input').setValue('shell');
    await vi.advanceTimersByTimeAsync(250);
    await wrapper.vm.$nextTick();

    expect(GET).toHaveBeenCalledTimes(1);
    expect(wrapper.text()).toContain('Shell doc');
  });

  it('navigates entries with arrow keys and selects with enter', async () => {
    const wrapper = mountPalette();
    const input = wrapper.get('input');

    await input.trigger('keydown', { key: 'ArrowDown' });
    await input.trigger('keydown', { key: 'Enter' });

    expect(wrapper.emitted('select')).toBeTruthy();
    const payload = wrapper.emitted('select')?.[0]?.[0] as { type: string; action?: { id: string } };
    expect(payload.type).toBe('action');
    expect(payload.action?.id).toBe('goto-tasks');
  });

  it('selects a search hit on enter and emits a hit payload', async () => {
    GET.mockResolvedValueOnce({
      data: {
        items: [
          {
            id: 't1',
            kind: 'task',
            title: 'A task',
            readable_id: 'ATL-7',
            score: 2,
            updated_at: '2026-01-01T00:00:00Z',
          },
        ],
        next_cursor: null,
        has_more: false,
      },
      error: undefined,
    });

    const wrapper = mountPalette();
    // "report" matches no local action, so the only entry is the server hit.
    await wrapper.get('input').setValue('report');
    await vi.advanceTimersByTimeAsync(250);
    await wrapper.vm.$nextTick();

    await wrapper.get('input').trigger('keydown', { key: 'Enter' });

    const payload = wrapper.emitted('select')?.[0]?.[0] as { type: string; hit?: { readable_id?: string } };
    expect(payload.type).toBe('hit');
    expect(payload.hit?.readable_id).toBe('ATL-7');
  });

  it('emits close on Escape', async () => {
    const wrapper = mountPalette();
    await wrapper.get('input').trigger('keydown', { key: 'Escape' });
    expect(wrapper.emitted('close')).toBeTruthy();
  });

  it('shows an empty-state when a query returns no hits', async () => {
    const wrapper = mountPalette();
    const store = useSearchStore();
    await wrapper.get('input').setValue('zzzqqq');
    await vi.advanceTimersByTimeAsync(250);
    await wrapper.vm.$nextTick();

    expect(store.results).toHaveLength(0);
    expect(wrapper.text()).toContain('No results');
  });
});
