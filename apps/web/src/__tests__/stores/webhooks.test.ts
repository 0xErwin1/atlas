import { createPinia, setActivePinia } from 'pinia';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { GET, POST, PATCH, DELETE } = vi.hoisted(() => ({
  GET: vi.fn(),
  POST: vi.fn(),
  PATCH: vi.fn(),
  DELETE: vi.fn(),
}));

vi.mock('@/api/wrapper', () => ({
  wrappedClient: { GET, POST, PATCH, DELETE },
}));

import type { IntegrationConfigDto, WebhookDto } from '@/stores/webhooks';
import { useWebhooksStore } from '@/stores/webhooks';

const webhook = (over: Partial<WebhookDto> = {}): WebhookDto => ({
  id: 'wh1',
  event_types: ['task.created'],
  is_active: true,
  scope_type: 'workspace',
  target_url: 'https://example.com/hook',
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
  workspace_id: 'ws-1',
  ...over,
});

const integration = (over: Partial<IntegrationConfigDto> = {}): IntegrationConfigDto => ({
  id: 'ic1',
  integration: 'github',
  integration_api_key_id: 'key-1',
  is_active: true,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
  workspace_id: 'ws-1',
  ...over,
});

describe('useWebhooksStore', () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it('loadWebhooks GETs the page and stores its items', async () => {
    GET.mockResolvedValueOnce({
      data: { items: [webhook()], has_more: false, next_cursor: null },
      error: undefined,
    });

    const store = useWebhooksStore();
    await store.loadWebhooks('ws');

    expect(GET).toHaveBeenCalledWith('/v1/workspaces/{ws}/webhooks', {
      params: { path: { ws: 'ws' } },
    });
    expect(store.webhooks.map((w) => w.id)).toEqual(['wh1']);
  });

  it('createWebhook POSTs the body, returns the secret dto and caches it without the secret', async () => {
    const created = { ...webhook({ id: 'wh2' }), secret: 'whsec_abc' };
    POST.mockResolvedValueOnce({ data: created, error: undefined });

    const store = useWebhooksStore();
    const result = await store.createWebhook('ws', {
      target_url: 'https://example.com/hook',
      event_types: ['task.created'],
      label: null,
      scope_type: 'workspace',
    });

    expect(result?.secret).toBe('whsec_abc');
    expect(POST).toHaveBeenCalledWith('/v1/workspaces/{ws}/webhooks', {
      params: { path: { ws: 'ws' } },
      body: {
        target_url: 'https://example.com/hook',
        event_types: ['task.created'],
        label: null,
        scope_type: 'workspace',
      },
    });
    const cached = store.webhooks.find((w) => w.id === 'wh2');
    expect(cached).toBeDefined();
    expect((cached as Record<string, unknown>).secret).toBeUndefined();
  });

  it('updateWebhook PATCHes the is_active toggle and replaces the cached webhook', async () => {
    GET.mockResolvedValueOnce({
      data: { items: [webhook({ is_active: true })], has_more: false, next_cursor: null },
      error: undefined,
    });
    PATCH.mockResolvedValueOnce({ data: webhook({ is_active: false }), error: undefined });

    const store = useWebhooksStore();
    await store.loadWebhooks('ws');

    const ok = await store.updateWebhook('ws', 'wh1', { is_active: false });

    expect(ok).toBe(true);
    expect(PATCH).toHaveBeenCalledWith('/v1/workspaces/{ws}/webhooks/{webhook_id}', {
      params: { path: { ws: 'ws', webhook_id: 'wh1' } },
      body: { is_active: false },
    });
    expect(store.webhooks.find((w) => w.id === 'wh1')?.is_active).toBe(false);
  });

  it('deleteWebhook DELETEs and drops it from the cache', async () => {
    GET.mockResolvedValueOnce({
      data: {
        items: [webhook({ id: 'wh1' }), webhook({ id: 'wh2' })],
        has_more: false,
        next_cursor: null,
      },
      error: undefined,
    });
    DELETE.mockResolvedValueOnce({ error: undefined });

    const store = useWebhooksStore();
    await store.loadWebhooks('ws');

    const ok = await store.deleteWebhook('ws', 'wh1');

    expect(ok).toBe(true);
    expect(DELETE).toHaveBeenCalledWith('/v1/workspaces/{ws}/webhooks/{webhook_id}', {
      params: { path: { ws: 'ws', webhook_id: 'wh1' } },
    });
    expect(store.webhooks.map((w) => w.id)).toEqual(['wh2']);
  });

  it('loadDeliveries GETs the page and stores its items', async () => {
    GET.mockResolvedValueOnce({
      data: {
        items: [
          {
            id: 'd1',
            attempt_no: 1,
            created_at: '2026-01-01T00:00:00Z',
            outbox_event_id: 'ev1',
            outcome: 'success',
            status_code: 200,
            subscription_id: 'wh1',
          },
        ],
        has_more: false,
        next_cursor: null,
      },
      error: undefined,
    });

    const store = useWebhooksStore();
    await store.loadDeliveries('ws', 'wh1');

    expect(GET).toHaveBeenCalledWith('/v1/workspaces/{ws}/webhooks/{webhook_id}/deliveries', {
      params: { path: { ws: 'ws', webhook_id: 'wh1' } },
    });
    expect(store.deliveries.map((d) => d.id)).toEqual(['d1']);
  });

  it('loadIntegrations GETs the plain array', async () => {
    GET.mockResolvedValueOnce({ data: [integration()], error: undefined });

    const store = useWebhooksStore();
    await store.loadIntegrations('ws');

    expect(GET).toHaveBeenCalledWith('/v1/workspaces/{ws}/integration-configs', {
      params: { path: { ws: 'ws' } },
    });
    expect(store.integrations.map((i) => i.id)).toEqual(['ic1']);
  });

  it('createIntegration POSTs github, returns the secret dto and caches it without the secret', async () => {
    const created = { ...integration({ id: 'ic2' }), secret: 'integ_xyz' };
    POST.mockResolvedValueOnce({ data: created, error: undefined });

    const store = useWebhooksStore();
    const result = await store.createIntegration('ws', 'github');

    expect(result?.secret).toBe('integ_xyz');
    expect(POST).toHaveBeenCalledWith('/v1/workspaces/{ws}/integration-configs', {
      params: { path: { ws: 'ws' } },
      body: { integration: 'github' },
    });
    const cached = store.integrations.find((i) => i.id === 'ic2');
    expect(cached).toBeDefined();
    expect((cached as Record<string, unknown>).secret).toBeUndefined();
  });

  it('setIntegrationActive PATCHes is_active and replaces the cached config', async () => {
    GET.mockResolvedValueOnce({ data: [integration({ is_active: true })], error: undefined });
    PATCH.mockResolvedValueOnce({ data: integration({ is_active: false }), error: undefined });

    const store = useWebhooksStore();
    await store.loadIntegrations('ws');

    const ok = await store.setIntegrationActive('ws', 'ic1', false);

    expect(ok).toBe(true);
    expect(PATCH).toHaveBeenCalledWith('/v1/workspaces/{ws}/integration-configs/{config_id}', {
      params: { path: { ws: 'ws', config_id: 'ic1' } },
      body: { is_active: false },
    });
    expect(store.integrations.find((i) => i.id === 'ic1')?.is_active).toBe(false);
  });

  it('deleteIntegration DELETEs and drops it from the cache', async () => {
    GET.mockResolvedValueOnce({
      data: [integration({ id: 'ic1' }), integration({ id: 'ic2' })],
      error: undefined,
    });
    DELETE.mockResolvedValueOnce({ error: undefined });

    const store = useWebhooksStore();
    await store.loadIntegrations('ws');

    const ok = await store.deleteIntegration('ws', 'ic1');

    expect(ok).toBe(true);
    expect(DELETE).toHaveBeenCalledWith('/v1/workspaces/{ws}/integration-configs/{config_id}', {
      params: { path: { ws: 'ws', config_id: 'ic1' } },
    });
    expect(store.integrations.map((i) => i.id)).toEqual(['ic2']);
  });

  it('updateWebhook returns false and sets error on failure', async () => {
    PATCH.mockResolvedValueOnce({ data: undefined, error: { hint: 'nope' } });

    const store = useWebhooksStore();
    const ok = await store.updateWebhook('ws', 'wh1', { is_active: false });

    expect(ok).toBe(false);
    expect(store.error).toBe('nope');
  });
});
