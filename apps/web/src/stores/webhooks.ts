import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import { errorHint } from '@/lib/apiError';

export type WebhookDto = components['schemas']['WebhookDto'];
export type WebhookCreatedDto = components['schemas']['WebhookCreatedDto'];
export type CreateWebhookRequest = components['schemas']['CreateWebhookRequest'];
export type WebhookDeliveryDto = components['schemas']['WebhookDeliveryDto'];
export type IntegrationConfigDto = components['schemas']['IntegrationConfigDto'];
export type IntegrationConfigCreatedDto = components['schemas']['IntegrationConfigCreatedDto'];

/** Fields a webhook PATCH may change; omitted fields are left unchanged server-side. */
export interface WebhookPatch {
  target_url?: string;
  event_types?: string[];
  is_active?: boolean;
  label?: string | null;
}

/**
 * Drops the one-time plaintext secret from a freshly created subscription so the
 * remaining fields can live in the cached `WebhookDto` list, which never carries it.
 */
function toWebhookDto(created: WebhookCreatedDto): WebhookDto {
  const { secret: _secret, ...rest } = created;
  return rest;
}

/** Same one-time-secret stripping for a freshly created integration config. */
function toIntegrationDto(created: IntegrationConfigCreatedDto): IntegrationConfigDto {
  const { secret: _secret, ...rest } = created;
  return rest;
}

export const useWebhooksStore = defineStore('webhooks', () => {
  const webhooks = ref<WebhookDto[]>([]);
  const integrations = ref<IntegrationConfigDto[]>([]);
  const deliveries = ref<WebhookDeliveryDto[]>([]);
  const error = ref<string | null>(null);

  async function loadWebhooks(ws: string): Promise<void> {
    error.value = null;

    const { data, error: apiError } = await wrappedClient.GET('/v1/workspaces/{ws}/webhooks', {
      params: { path: { ws } },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to load webhooks');
      return;
    }

    webhooks.value = data.items;
  }

  async function createWebhook(ws: string, body: CreateWebhookRequest): Promise<WebhookCreatedDto | null> {
    error.value = null;

    const { data, error: apiError } = await wrappedClient.POST('/v1/workspaces/{ws}/webhooks', {
      params: { path: { ws } },
      body,
    });

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to create webhook');
      return null;
    }

    webhooks.value = [...webhooks.value, toWebhookDto(data)];
    return data;
  }

  async function updateWebhook(ws: string, id: string, patch: WebhookPatch): Promise<boolean> {
    error.value = null;

    const { data, error: apiError } = await wrappedClient.PATCH('/v1/workspaces/{ws}/webhooks/{webhook_id}', {
      params: { path: { ws, webhook_id: id } },
      body: patch,
    });

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to update webhook');
      return false;
    }

    webhooks.value = webhooks.value.map((w) => (w.id === id ? data : w));
    return true;
  }

  async function deleteWebhook(ws: string, id: string): Promise<boolean> {
    error.value = null;

    const { error: apiError } = await wrappedClient.DELETE('/v1/workspaces/{ws}/webhooks/{webhook_id}', {
      params: { path: { ws, webhook_id: id } },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to delete webhook');
      return false;
    }

    webhooks.value = webhooks.value.filter((w) => w.id !== id);
    return true;
  }

  async function loadDeliveries(ws: string, id: string): Promise<void> {
    error.value = null;

    const { data, error: apiError } = await wrappedClient.GET(
      '/v1/workspaces/{ws}/webhooks/{webhook_id}/deliveries',
      { params: { path: { ws, webhook_id: id } } },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to load deliveries');
      return;
    }

    deliveries.value = data.items;
  }

  async function loadIntegrations(ws: string): Promise<void> {
    error.value = null;

    const { data, error: apiError } = await wrappedClient.GET('/v1/workspaces/{ws}/integration-configs', {
      params: { path: { ws } },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to load integrations');
      return;
    }

    integrations.value = data;
  }

  async function createIntegration(
    ws: string,
    integration: string,
  ): Promise<IntegrationConfigCreatedDto | null> {
    error.value = null;

    const { data, error: apiError } = await wrappedClient.POST('/v1/workspaces/{ws}/integration-configs', {
      params: { path: { ws } },
      body: { integration },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to create integration');
      return null;
    }

    integrations.value = [...integrations.value, toIntegrationDto(data)];
    return data;
  }

  async function setIntegrationActive(ws: string, id: string, isActive: boolean): Promise<boolean> {
    error.value = null;

    const { data, error: apiError } = await wrappedClient.PATCH(
      '/v1/workspaces/{ws}/integration-configs/{config_id}',
      {
        params: { path: { ws, config_id: id } },
        body: { is_active: isActive },
      },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to update integration');
      return false;
    }

    integrations.value = integrations.value.map((i) => (i.id === id ? data : i));
    return true;
  }

  async function deleteIntegration(ws: string, id: string): Promise<boolean> {
    error.value = null;

    const { error: apiError } = await wrappedClient.DELETE(
      '/v1/workspaces/{ws}/integration-configs/{config_id}',
      { params: { path: { ws, config_id: id } } },
    );

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to delete integration');
      return false;
    }

    integrations.value = integrations.value.filter((i) => i.id !== id);
    return true;
  }

  return {
    webhooks,
    integrations,
    deliveries,
    error,
    loadWebhooks,
    createWebhook,
    updateWebhook,
    deleteWebhook,
    loadDeliveries,
    loadIntegrations,
    createIntegration,
    setIntegrationActive,
    deleteIntegration,
  };
});
