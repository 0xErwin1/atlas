import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import { errorHint } from '@/lib/apiError';

export type PlatformStatusTemplateDto = components['schemas']['PlatformStatusTemplateDto'];

/**
 * Atlas-wide default statuses: the instance-level list every newly created
 * workspace copies into its own status templates. Mirrors the workspace
 * `statusTemplates` store minus the workspace binding — these rows belong to the
 * Atlas instance, so there is nothing to scope or reset per workspace. The
 * surface is admin-only (root / system admin); a non-admin caller gets a 403 the
 * store reports through `error`.
 */
export const usePlatformStatusTemplatesStore = defineStore('platformStatusTemplates', () => {
  const templates = ref<PlatformStatusTemplateDto[]>([]);
  const error = ref<string | null>(null);

  function bySortedPosition(list: PlatformStatusTemplateDto[]): PlatformStatusTemplateDto[] {
    return [...list].sort((a, b) => a.position_key.localeCompare(b.position_key));
  }

  async function load(): Promise<void> {
    const { data, error: apiError } = await wrappedClient.GET('/api/v2/acta/admin/status-templates');

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to load Atlas default statuses');
      return;
    }

    error.value = null;
    templates.value = bySortedPosition(data);
  }

  /**
   * Creates a default status appended after the current last one, then inserts it
   * into the sorted cache. Returns the created row, or null on failure.
   */
  async function create(name: string): Promise<PlatformStatusTemplateDto | null> {
    const { data, error: apiError } = await wrappedClient.POST('/api/v2/acta/admin/status-templates', {
      body: { name, before: null, after: null },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to create Atlas default status');
      return null;
    }

    error.value = null;
    templates.value = bySortedPosition([...templates.value, data]);
    return data;
  }

  /**
   * Patches a default status's name and/or color (a swatch id or a #RRGGBB hex;
   * `null` clears it). Returns true on success.
   */
  async function update(id: string, patch: { name?: string; color?: string | null }): Promise<boolean> {
    const { data, error: apiError } = await wrappedClient.PATCH(
      '/api/v2/acta/admin/status-templates/{template_id}',
      {
        params: { path: { template_id: id } },
        body: patch,
      },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to update Atlas default status');
      return false;
    }

    error.value = null;
    templates.value = bySortedPosition(templates.value.map((t) => (t.id === id ? data : t)));
    return true;
  }

  /**
   * Reorders a default status by requesting a new position between `before`/`after`
   * sibling position keys. Returns true on success.
   */
  async function move(
    id: string,
    placement: { before: string | null; after: string | null },
  ): Promise<boolean> {
    const { data, error: apiError } = await wrappedClient.PATCH(
      '/api/v2/acta/admin/status-templates/{template_id}',
      {
        params: { path: { template_id: id } },
        body: { before: placement.before, after: placement.after },
      },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to reorder Atlas default status');
      return false;
    }

    error.value = null;
    templates.value = bySortedPosition(templates.value.map((t) => (t.id === id ? data : t)));
    return true;
  }

  /** Deletes a default status and drops it from the cache. Returns true on success. */
  async function remove(id: string): Promise<boolean> {
    const { error: apiError } = await wrappedClient.DELETE(
      '/api/v2/acta/admin/status-templates/{template_id}',
      {
        params: { path: { template_id: id } },
      },
    );

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to delete Atlas default status');
      return false;
    }

    error.value = null;
    templates.value = templates.value.filter((t) => t.id !== id);
    return true;
  }

  return {
    templates,
    error,
    load,
    create,
    update,
    move,
    remove,
  };
});
