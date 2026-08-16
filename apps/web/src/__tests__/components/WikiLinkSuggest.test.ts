import { flushPromises, mount } from '@vue/test-utils';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET } = vi.hoisted(() => ({ GET: vi.fn() }));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET },
}));

import WikiLinkSuggest from '@/components/notas/WikiLinkSuggest.vue';

const noteHit = (id: string, title: string, slug: string) => ({
  id,
  title,
  kind: 'note',
  document_slug: slug,
  score: 1,
  updated_at: '2026-01-01T00:00:00Z',
});

const taskHit = (id: string, title: string, readableId: string) => ({
  id,
  title,
  kind: 'task',
  readable_id: readableId,
  score: 1,
  updated_at: '2026-01-01T00:00:00Z',
});

type AttachmentOwner = InstanceType<typeof WikiLinkSuggest>['$props']['attachmentOwner'];

function typeQuery(query: string, attachmentOwner?: AttachmentOwner) {
  return mount(WikiLinkSuggest, { props: { ws: 'ws', query, attachmentOwner } });
}

/** The `type` filter the component asked the search endpoint for. */
function requestedType(): unknown {
  return GET.mock.calls[0]?.[1]?.params?.query?.type;
}

describe('WikiLinkSuggest', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('is hidden when there is no active query', () => {
    GET.mockResolvedValue({ data: { items: [], has_more: false } });
    const wrapper = mount(WikiLinkSuggest, { props: { ws: 'ws', query: null } });
    expect(wrapper.find('[role="listbox"]').exists()).toBe(false);
  });

  it('offers notes and tasks together for an unscoped query', async () => {
    GET.mockResolvedValue({
      data: {
        items: [noteHit('d1', 'Architecture', 'architecture'), taskHit('t1', 'Fix login', 'ATL-80')],
        has_more: false,
      },
    });

    const wrapper = typeQuery('arch');
    await flushPromises();

    expect(requestedType()).toBe('note,task');
    expect(wrapper.text()).toContain('Architecture');
    expect(wrapper.text()).toContain('Fix login');
    expect(wrapper.text()).toContain('ATL-80');
  });

  it('scopes the search to the kind named by the typed prefix', async () => {
    GET.mockResolvedValue({ data: { items: [taskHit('t1', 'Fix login', 'ATL-80')], has_more: false } });

    typeQuery('task:log');
    await flushPromises();

    expect(requestedType()).toBe('task');
    expect(GET.mock.calls[0]?.[1]?.params?.query?.q).toBe('log');
  });

  it('emits a typed note reference addressed by slug', async () => {
    GET.mockResolvedValue({
      data: { items: [noteHit('d1', 'Architecture', 'architecture')], has_more: false },
    });

    const wrapper = typeQuery('arch');
    await flushPromises();

    const option = wrapper.findAll('[role="option"]').find((o) => o.text().includes('Architecture'));
    await option?.trigger('mousedown');

    expect(wrapper.emitted('select')?.[0]).toEqual([
      { target: { kind: 'note', slug: 'architecture' }, display: 'Architecture' },
    ]);
  });

  it('emits a typed task reference addressed by readable id', async () => {
    GET.mockResolvedValue({ data: { items: [taskHit('t1', 'Fix login', 'ATL-80')], has_more: false } });

    const wrapper = typeQuery('fix');
    await flushPromises();

    const option = wrapper.findAll('[role="option"]').find((o) => o.text().includes('Fix login'));
    await option?.trigger('mousedown');

    expect(wrapper.emitted('select')?.[0]).toEqual([
      { target: { kind: 'task', readableId: 'ATL-80' }, display: 'Fix login' },
    ]);
  });

  it('lists the owning resource attachments for a file query', async () => {
    GET.mockResolvedValue({
      data: [
        { id: 'a1', file_name: 'policy.pdf' },
        { id: 'a2', file_name: 'notes.txt' },
      ],
    });

    const wrapper = typeQuery('file:pol', { kind: 'document', slug: 'runbook' });
    await flushPromises();

    expect(wrapper.text()).toContain('policy.pdf');
    expect(wrapper.text()).not.toContain('notes.txt');

    const option = wrapper.findAll('[role="option"]').find((o) => o.text().includes('policy.pdf'));
    await option?.trigger('mousedown');

    expect(wrapper.emitted('select')?.[0]).toEqual([
      { target: { kind: 'file', fileName: 'policy.pdf' }, display: 'policy.pdf' },
    ]);
  });

  it('does not offer creation for a kind that cannot be created from a title', async () => {
    GET.mockResolvedValue({ data: { items: [], has_more: false } });

    const wrapper = typeQuery('task:nothing-matches');
    await flushPromises();

    expect(wrapper.text()).not.toContain('Create');
  });

  it('degrades gracefully on a network error and still offers creation (REQ-W16)', async () => {
    GET.mockResolvedValue({ error: { status: 500 } });

    const wrapper = typeQuery('newnote');
    await flushPromises();

    expect(wrapper.text()).toContain('Search unavailable');

    const createOption = wrapper.findAll('[role="option"]').find((o) => o.text().includes('Create'));
    await createOption?.trigger('mousedown');

    expect(wrapper.emitted('select')?.[0]).toEqual([{ target: { kind: 'title' }, display: 'newnote' }]);
  });

  it('confirms the active item via keyboard navigation', async () => {
    GET.mockResolvedValue({
      data: {
        items: [noteHit('d1', 'Alpha', 'alpha'), noteHit('d2', 'Beta', 'beta')],
        has_more: false,
      },
    });

    const wrapper = typeQuery('a');
    await flushPromises();

    wrapper.vm.moveDown();
    wrapper.vm.confirmActive();

    expect(wrapper.emitted('select')?.[0]).toEqual([
      { target: { kind: 'note', slug: 'beta' }, display: 'Beta' },
    ]);
  });
});
