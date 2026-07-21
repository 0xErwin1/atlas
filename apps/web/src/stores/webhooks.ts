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
  let workspaceGeneration = 0;
  let boundWorkspace: string | null = null;

  function resetWorkspace(): void {
    workspaceGeneration += 1;
    boundWorkspace = null;
    webhooks.value = [];
    integrations.value = [];
    deliveries.value = [];
    error.value = null;
  }

  function bindWorkspace(ws: string): number {
    if (boundWorkspace !== ws) resetWorkspace();
    boundWorkspace = ws;
    return workspaceGeneration;
  }

  function isCurrentWorkspace(ws: string, generation: number): boolean {
    return boundWorkspace === ws && workspaceGeneration === generation;
  }

  async function loadWebhooks(ws: string): Promise<void> {
    const generation = bindWorkspace(ws);
    error.value = null;

    const { data, error: apiError } = await wrappedClient.GET('/api/workspaces/{ws}/webhooks', {
      params: { path: { ws } },
    });

    if (!isCurrentWorkspace(ws, generation)) return;
    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to load webhooks');
      return;
    }

    webhooks.value = data.items;
  }

  async function createWebhook(ws: string, body: CreateWebhookRequest): Promise<WebhookCreatedDto | null> {
    const generation = bindWorkspace(ws);
    error.value = null;

    const { data, error: apiError } = await wrappedClient.POST('/api/workspaces/{ws}/webhooks', {
      params: { path: { ws } },
      body,
    });

    if (!isCurrentWorkspace(ws, generation)) return data ?? null;
    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to create webhook');
      return null;
    }

    webhooks.value = [...webhooks.value, toWebhookDto(data)];
    return data;
  }

  async function updateWebhook(ws: string, id: string, patch: WebhookPatch): Promise<boolean> {
    const generation = bindWorkspace(ws);
    error.value = null;

    const { data, error: apiError } = await wrappedClient.PATCH(
      '/api/workspaces/{ws}/webhooks/{webhook_id}',
      {
        params: { path: { ws, webhook_id: id } },
        body: patch,
      },
    );

    if (!isCurrentWorkspace(ws, generation)) return apiError === undefined && data !== undefined;
    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to update webhook');
      return false;
    }

    webhooks.value = webhooks.value.map((w) => (w.id === id ? data : w));
    return true;
  }

  async function deleteWebhook(ws: string, id: string): Promise<boolean> {
    const generation = bindWorkspace(ws);
    error.value = null;

    const { error: apiError } = await wrappedClient.DELETE('/api/workspaces/{ws}/webhooks/{webhook_id}', {
      params: { path: { ws, webhook_id: id } },
    });

    if (!isCurrentWorkspace(ws, generation)) return apiError === undefined;
    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to delete webhook');
      return false;
    }

    webhooks.value = webhooks.value.filter((w) => w.id !== id);
    return true;
  }

  async function loadDeliveries(ws: string, id: string): Promise<void> {
    const generation = bindWorkspace(ws);
    error.value = null;

    const { data, error: apiError } = await wrappedClient.GET(
      '/api/workspaces/{ws}/webhooks/{webhook_id}/deliveries',
      { params: { path: { ws, webhook_id: id } } },
    );

    if (!isCurrentWorkspace(ws, generation)) return;
    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to load deliveries');
      return;
    }

    deliveries.value = data.items;
  }

  async function loadIntegrations(ws: string): Promise<void> {
    const generation = bindWorkspace(ws);
    error.value = null;

    const { data, error: apiError } = await wrappedClient.GET('/api/workspaces/{ws}/integration-configs', {
      params: { path: { ws } },
    });

    if (!isCurrentWorkspace(ws, generation)) return;
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
    const generation = bindWorkspace(ws);
    error.value = null;

    const { data, error: apiError } = await wrappedClient.POST('/api/workspaces/{ws}/integration-configs', {
      params: { path: { ws } },
      body: { integration },
    });

    if (!isCurrentWorkspace(ws, generation)) return data ?? null;
    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to create integration');
      return null;
    }

    integrations.value = [...integrations.value, toIntegrationDto(data)];
    return data;
  }

  async function setIntegrationActive(ws: string, id: string, isActive: boolean): Promise<boolean> {
    const generation = bindWorkspace(ws);
    error.value = null;

    const { data, error: apiError } = await wrappedClient.PATCH(
      '/api/workspaces/{ws}/integration-configs/{config_id}',
      {
        params: { path: { ws, config_id: id } },
        body: { is_active: isActive },
      },
    );

    if (!isCurrentWorkspace(ws, generation)) return apiError === undefined && data !== undefined;
    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to update integration');
      return false;
    }

    integrations.value = integrations.value.map((i) => (i.id === id ? data : i));
    return true;
  }

  async function deleteIntegration(ws: string, id: string): Promise<boolean> {
    const generation = bindWorkspace(ws);
    error.value = null;

    const { error: apiError } = await wrappedClient.DELETE(
      '/api/workspaces/{ws}/integration-configs/{config_id}',
      { params: { path: { ws, config_id: id } } },
    );

    if (!isCurrentWorkspace(ws, generation)) return apiError === undefined;
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
    resetWorkspace,
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
