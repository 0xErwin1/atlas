import { mount } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET, POST, DELETE } = vi.hoisted(() => ({
  GET: vi.fn(),
  POST: vi.fn(),
  DELETE: vi.fn(),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET, POST, DELETE },
}));

import ShareDialog from '@/components/share/ShareDialog.vue';
import { useShareStore } from '@/stores/share';

const grant = (id: string, type: 'user' | 'api_key', principalId: string, role: string) => ({
  id,
  principal: { type, id: principalId },
  role,
  created_at: '2026-01-01T00:00:00Z',
});

const mountDialog = () =>
  mount(ShareDialog, {
    props: {
      open: true,
      ws: 'acme',
      resourceLabel: 'PRD — Atlas · note',
      visibility: 'workspace',
    },
  });

const member = (type: 'user' | 'api_key', id: string, display: string) => ({
  principal_type: type,
  id,
  display,
});

describe('ShareDialog (REQ-W26/W27)', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    GET.mockImplementation((path: string) => {
      if (path === '/v1/workspaces/{ws}/members') {
        return Promise.resolve({
          data: [
            member('user', 'u1', 'Ada Lovelace'),
            member('user', 'u9', 'Grace Hopper'),
            member('api_key', 'k2', 'ci-bot'),
          ],
        });
      }
      if (path === '/v1/api-keys') {
        return Promise.resolve({
          data: { items: [], has_more: false },
        });
      }
      if (path === '/v1/workspaces/{ws}/groups') {
        return Promise.resolve({ data: [] });
      }
      return Promise.resolve({
        data: {
          items: [grant('g1', 'user', 'u1', 'editor'), grant('g2', 'api_key', 'k1', 'editor')],
          has_more: false,
        },
      });
    });
  });

  it('loads and lists grants when opened', async () => {
    const wrapper = mountDialog();
    await vi.waitFor(() => expect(useShareStore().grants).toHaveLength(2));
    await wrapper.vm.$nextTick();

    const rows = wrapper.findAll('[data-grant-row]');
    expect(rows).toHaveLength(2);
  });

  it('marks the agent grant with the agent badge', async () => {
    const wrapper = mountDialog();
    await vi.waitFor(() => expect(useShareStore().grants).toHaveLength(2));
    await wrapper.vm.$nextTick();

    const agentRow = wrapper.find('[data-grant-row][data-principal-type="api_key"]');
    expect(agentRow.exists()).toBe(true);
    expect(agentRow.findComponent({ name: 'AgentBadge' }).exists()).toBe(true);
  });

  it('lists the public option but keeps visibility read-only: clicking does not emit or persist', async () => {
    const wrapper = mountDialog();
    await wrapper.vm.$nextTick();

    expect(wrapper.find('[data-visibility="public"]').exists()).toBe(true);

    const privateOpt = wrapper.find('[data-visibility="private"]');
    await privateOpt.trigger('click');

    const publicOpt = wrapper.find('[data-visibility="public"]');
    await publicOpt.trigger('click');

    expect(wrapper.emitted('update:visibility')).toBeUndefined();
    expect(POST).not.toHaveBeenCalled();
  });

  it('filters loaded members by display and adds a viewer grant on selection', async () => {
    const wrapper = mountDialog();
    await vi.waitFor(() => expect(useShareStore().members).toHaveLength(3));
    await wrapper.vm.$nextTick();

    const input = wrapper.find('[data-member-search]');
    await input.setValue('grace');
    await wrapper.vm.$nextTick();

    const options = wrapper.findAll('[data-member-option]');
    expect(options).toHaveLength(1);
    expect(options[0]?.text()).toContain('Grace Hopper');

    POST.mockResolvedValue({ data: grant('g3', 'user', 'u9', 'viewer') });
    await options[0]?.trigger('click');

    expect(POST).toHaveBeenCalledWith('/v1/workspaces/{ws}/grants', {
      params: { path: { ws: 'acme' } },
      body: { principal: { type: 'user', id: 'u9' }, role: 'viewer' },
    });
  });

  it('excludes principals that already have a grant from the typeahead matches', async () => {
    const wrapper = mountDialog();
    await vi.waitFor(() => expect(useShareStore().members).toHaveLength(3));
    await wrapper.vm.$nextTick();

    // u1 already holds grant g1, so searching "ada" (u1) yields no match.
    await wrapper.find('[data-member-search]').setValue('ada');
    await wrapper.vm.$nextTick();

    expect(wrapper.findAll('[data-member-option]')).toHaveLength(0);
  });

  it('emits close when the close button is clicked', async () => {
    const wrapper = mountDialog();
    await wrapper.vm.$nextTick();

    await wrapper.find('[data-action="close"]').trigger('click');

    expect(wrapper.emitted('close')).toBeTruthy();
  });

  it('surfaces the store error hint', async () => {
    GET.mockResolvedValue({ error: { hint: 'no access to grants' } });

    const wrapper = mountDialog();
    await vi.waitFor(() => expect(useShareStore().error).toBe('no access to grants'));
    await wrapper.vm.$nextTick();

    expect(wrapper.text()).toContain('no access to grants');
  });
});
