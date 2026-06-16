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

describe('ShareDialog (REQ-W26/W27)', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
    GET.mockResolvedValue({
      data: {
        items: [grant('g1', 'user', 'u1', 'editor'), grant('g2', 'api_key', 'k1', 'editor')],
        has_more: false,
      },
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

  it('emits update:visibility when a visibility option is chosen', async () => {
    const wrapper = mountDialog();
    await wrapper.vm.$nextTick();

    await wrapper.find('[data-visibility="private"]').trigger('click');

    expect(wrapper.emitted('update:visibility')?.[0]).toEqual(['private']);
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
