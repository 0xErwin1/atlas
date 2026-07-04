import { flushPromises, mount } from '@vue/test-utils';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const { GET } = vi.hoisted(() => ({
  GET: vi.fn(),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET },
}));

import ReferenceAdd from '@/components/tareas/ReferenceAdd.vue';

const HITS = [
  { id: 't1', kind: 'task', readable_id: 'ATL-56', title: 'The current task' },
  { id: 't2', kind: 'task', readable_id: 'ATL-57', title: 'Another task' },
];

function mountAdd(currentReadableId?: string) {
  return mount(ReferenceAdd, {
    props: { ws: 'atlas', currentReadableId },
    global: { stubs: { Dropdown: true, Icon: true } },
  });
}

async function search(wrapper: ReturnType<typeof mountAdd>, term: string) {
  await wrapper.get('.atl-refadd-input').setValue(term);
  await vi.advanceTimersByTimeAsync(250);
  await flushPromises();
}

describe('ReferenceAdd', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    GET.mockResolvedValue({ data: { items: HITS } });
  });

  afterEach(() => {
    vi.useRealTimers();
    GET.mockReset();
  });

  it('excludes the current task from the search results', async () => {
    const wrapper = mountAdd('ATL-56');

    await search(wrapper, 'task');

    const results = wrapper.findAll('.atl-refadd-result');
    expect(results).toHaveLength(1);
    expect(results[0]?.text()).toContain('ATL-57');
    expect(wrapper.text()).not.toContain('ATL-56');
  });

  it('lists every task when no current task is set to exclude', async () => {
    const wrapper = mountAdd(undefined);

    await search(wrapper, 'task');

    expect(wrapper.findAll('.atl-refadd-result')).toHaveLength(2);
  });
});
