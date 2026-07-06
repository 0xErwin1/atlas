import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import { errorHint } from '@/lib/apiError';

export type PropertyDefinitionDto = components['schemas']['PropertyDefinitionDto'];
export type CreatePropertyDefinitionRequest = components['schemas']['CreatePropertyDefinitionRequest'];

/**
 * Workspace custom-field definitions that apply to tasks. The list is cached per
 * workspace; task value editing reads it to know each field's kind and options.
 */
export const usePropertyDefinitionsStore = defineStore('propertyDefinitions', () => {
  const definitions = ref<PropertyDefinitionDto[]>([]);
  const loadedWs = ref<string | null>(null);
  const error = ref<string | null>(null);

  async function load(ws: string, force = false): Promise<void> {
    if (!force && loadedWs.value === ws) return;
    error.value = null;

    const { data, error: apiError } = await wrappedClient.GET('/api/workspaces/{ws}/property-definitions', {
      params: { path: { ws }, query: { applies_to: 'task' } },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to load custom fields');
      return;
    }

    definitions.value = data;
    loadedWs.value = ws;
  }

  async function create(
    ws: string,
    body: CreatePropertyDefinitionRequest,
  ): Promise<PropertyDefinitionDto | null> {
    error.value = null;

    const { data, error: apiError } = await wrappedClient.POST('/api/workspaces/{ws}/property-definitions', {
      params: { path: { ws } },
      body,
    });

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to create custom field');
      return null;
    }

    definitions.value = [...definitions.value, data];
    return data;
  }

  async function remove(ws: string, id: string): Promise<boolean> {
    error.value = null;

    const snapshot = [...definitions.value];
    definitions.value = definitions.value.filter((d) => d.id !== id);

    const { error: apiError } = await wrappedClient.DELETE(
      '/api/workspaces/{ws}/property-definitions/{property_definition_id}',
      { params: { path: { ws, property_definition_id: id } } },
    );

    if (apiError !== undefined) {
      definitions.value = snapshot;
      error.value = errorHint(apiError, 'Failed to delete custom field');
      return false;
    }

    return true;
  }

  return { definitions, error, load, create, remove };
});
