import { flushPromises, mount, type VueWrapper } from '@vue/test-utils';
import { createPinia, setActivePinia } from 'pinia';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import WebhooksPanel from '@/components/settings/WebhooksPanel.vue';
import MultiSelect from '@/components/ui/MultiSelect.vue';
import { type IntegrationConfigDto, useWebhooksStore, type WebhookDto } from '@/stores/webhooks';
import { useWorkspaceStore } from '@/stores/workspace';

function webhook(over: Partial<WebhookDto> = {}): WebhookDto {
  return {
    id: 'wh1',
    event_types: ['task.created'],
    is_active: true,
    scope_type: 'workspace',
    target_url: 'https://example.com/hook',
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    workspace_id: 'ws-1',
    ...over,
  };
}

function integration(over: Partial<IntegrationConfigDto> = {}): IntegrationConfigDto {
  return {
    id: 'ic1',
    integration: 'github',
    integration_api_key_id: 'key-1',
    is_active: true,
    created_at: '2026-01-01T00:00:00Z',
    updated_at: '2026-01-01T00:00:00Z',
    workspace_id: 'ws-1',
    ...over,
  };
}

function setup(webhooks: WebhookDto[], integrations: IntegrationConfigDto[]) {
  setActivePinia(createPinia());

  const ws = useWorkspaceStore();
  ws.activeWorkspaceSlug = 'acme';

  const store = useWebhooksStore();
  store.webhooks = webhooks;
  store.integrations = integrations;
  vi.spyOn(store, 'loadWebhooks').mockResolvedValue(undefined);
  vi.spyOn(store, 'loadIntegrations').mockResolvedValue(undefined);
  vi.spyOn(store, 'loadDeliveries').mockResolvedValue(undefined);

  return store;
}

function findBtn(wrapper: VueWrapper, text: string) {
  const btn = wrapper.findAll('button').find((b) => b.text().includes(text));
  if (btn === undefined) throw new Error(`button not found: ${text}`);
  return btn;
}

let activeWrapper: VueWrapper | null = null;

async function mountPanel(): Promise<VueWrapper> {
  const wrapper = mount(WebhooksPanel, { attachTo: document.body });
  activeWrapper = wrapper;
  await flushPromises();
  return wrapper;
}

afterEach(() => {
  activeWrapper?.unmount();
  activeWrapper = null;
  document.body.innerHTML = '';
});

describe('WebhooksPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('lists webhooks and integrations', async () => {
    setup([webhook({ label: 'Deploy notifier' })], [integration()]);

    const wrapper = await mountPanel();

    expect(wrapper.text()).toContain('Deploy notifier');
    expect(wrapper.text()).toContain('https://example.com/hook');
    expect(wrapper.text()).toContain('task.created');
    expect(wrapper.text()).toContain('github');
  });

  it('toggling a webhook active switch calls updateWebhook with the negated value', async () => {
    const store = setup([webhook({ is_active: true })], []);
    const update = vi.spyOn(store, 'updateWebhook').mockResolvedValue(true);

    const wrapper = await mountPanel();

    await wrapper.find('[data-webhook-toggle]').trigger('click');

    expect(update).toHaveBeenCalledWith('acme', 'wh1', { is_active: false });
  });

  it('Add GitHub creates the integration and reveals the one-time secret', async () => {
    const store = setup([], []);
    const create = vi.spyOn(store, 'createIntegration').mockResolvedValue({
      ...integration({ id: 'ic9' }),
      secret: 'integ_secret',
    });

    const wrapper = await mountPanel();

    await findBtn(wrapper, 'Add GitHub').trigger('click');
    await flushPromises();

    expect(create).toHaveBeenCalledWith('acme', 'github');
    expect(wrapper.text()).toContain('integ_secret');
  });

  it('creating a webhook through the form calls createWebhook with the workspace scope', async () => {
    const store = setup([webhook()], []);
    const create = vi.spyOn(store, 'createWebhook').mockResolvedValue({
      ...webhook({ id: 'wh9' }),
      secret: 'whsec_secret',
    });

    const wrapper = await mountPanel();

    await findBtn(wrapper, 'Add webhook').trigger('click');
    await flushPromises();

    await wrapper.find('input').setValue('https://hooks.example.com/in');

    await wrapper.findComponent(MultiSelect).find('.cursor-pointer').trigger('click');
    await flushPromises();
    await wrapper.findAll('[role="option"]')[0]?.trigger('click');

    await findBtn(wrapper, 'Create webhook').trigger('click');
    await flushPromises();

    expect(create).toHaveBeenCalledWith('acme', {
      target_url: 'https://hooks.example.com/in',
      event_types: ['task.created'],
      label: null,
      scope_type: 'workspace',
    });
  });
});
