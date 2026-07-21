import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import { errorHint } from '@/lib/apiError';

export type StatusTemplateDto = components['schemas']['StatusTemplateDto'];

/**
 * Workspace status templates: the workspace-level set of default statuses that
 * new boards are seeded from and that can be applied to an existing board. They
 * mirror board columns one-to-one (name, color, fractional position_key) but are
 * not bound to any board — editing a template never retro-updates a board's
 * columns; apply/seed copy them. The mutation methods deliberately mirror the
 * boards store's column actions so the panels share the same row patterns.
 */
export const useStatusTemplatesStore = defineStore('statusTemplates', () => {
  const templates = ref<StatusTemplateDto[]>([]);
  const error = ref<string | null>(null);
  let workspaceGeneration = 0;
  let boundWorkspace: string | null = null;

  function bySortedPosition(list: StatusTemplateDto[]): StatusTemplateDto[] {
    return [...list].sort((a, b) => a.position_key.localeCompare(b.position_key));
  }

  function resetWorkspace(): void {
    workspaceGeneration += 1;
    boundWorkspace = null;
    templates.value = [];
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

  async function load(ws: string): Promise<void> {
    const generation = bindWorkspace(ws);
    const { data, error: apiError } = await wrappedClient.GET('/api/workspaces/{ws}/status-templates', {
      params: { path: { ws } },
    });

    if (!isCurrentWorkspace(ws, generation)) return;
    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to load status templates');
      return;
    }

    templates.value = bySortedPosition(data);
  }

  /**
   * Creates a template appended after the current last one (the new position is
   * requested between the last template and the end), then inserts it into the
   * sorted cache. Returns the created template, or null on failure.
   */
  async function create(ws: string, name: string): Promise<StatusTemplateDto | null> {
    const generation = bindWorkspace(ws);
    const last = templates.value.at(-1);

    const { data, error: apiError } = await wrappedClient.POST('/api/workspaces/{ws}/status-templates', {
      params: { path: { ws } },
      body: { name, before: last?.position_key ?? null, after: null },
    });

    if (!isCurrentWorkspace(ws, generation)) return data ?? null;
    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to create status template');
      return null;
    }

    templates.value = bySortedPosition([...templates.value, data]);
    return data;
  }

  /**
   * Patches a template's name and/or color (color is a swatch id or a #RRGGBB
   * hex; `null` clears it). Replaces the cached template, re-sorting so a name
   * change never disturbs the ordering. Returns true on success.
   */
  async function update(
    ws: string,
    id: string,
    patch: { name?: string; color?: string | null },
  ): Promise<boolean> {
    const generation = bindWorkspace(ws);
    const { data, error: apiError } = await wrappedClient.PATCH(
      '/api/workspaces/{ws}/status-templates/{template_id}',
      {
        params: { path: { ws, template_id: id } },
        body: patch,
      },
    );

    if (!isCurrentWorkspace(ws, generation)) return apiError === undefined && data !== undefined;
    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to update status template');
      return false;
    }

    templates.value = bySortedPosition(templates.value.map((t) => (t.id === id ? data : t)));
    return true;
  }

  /**
   * Reorders a template by requesting a new position between `before`/`after`
   * sibling position keys. Re-sorts the cache by the returned `position_key`.
   * Returns true on success.
   */
  async function move(
    ws: string,
    id: string,
    placement: { before: string | null; after: string | null },
  ): Promise<boolean> {
    const generation = bindWorkspace(ws);
    const { data, error: apiError } = await wrappedClient.PATCH(
      '/api/workspaces/{ws}/status-templates/{template_id}',
      {
        params: { path: { ws, template_id: id } },
        body: { before: placement.before, after: placement.after },
      },
    );

    if (!isCurrentWorkspace(ws, generation)) return apiError === undefined && data !== undefined;
    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to reorder status template');
      return false;
    }

    templates.value = bySortedPosition(templates.value.map((t) => (t.id === id ? data : t)));
    return true;
  }

  /** Deletes a template and drops it from the cache. Returns true on success. */
  async function remove(ws: string, id: string): Promise<boolean> {
    const generation = bindWorkspace(ws);
    const { error: apiError } = await wrappedClient.DELETE(
      '/api/workspaces/{ws}/status-templates/{template_id}',
      { params: { path: { ws, template_id: id } } },
    );

    if (!isCurrentWorkspace(ws, generation)) return apiError === undefined;
    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to delete status template');
      return false;
    }

    templates.value = templates.value.filter((t) => t.id !== id);
    return true;
  }

  /**
   * Applies the workspace's status templates to a board: the server adds a column
   * for each template whose name is not already present and is idempotent.
   * Returns true on success; the caller reloads the board's columns to reflect it.
   */
  async function applyToBoard(ws: string, boardId: string): Promise<boolean> {
    const generation = bindWorkspace(ws);
    const { error: apiError } = await wrappedClient.POST(
      '/api/workspaces/{ws}/boards/{board_id}/apply-status-templates',
      { params: { path: { ws, board_id: boardId } } },
    );

    if (!isCurrentWorkspace(ws, generation)) return apiError === undefined;
    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to apply status templates');
      return false;
    }

    return true;
  }

  return {
    templates,
    error,
    resetWorkspace,
    load,
    create,
    update,
    move,
    remove,
    applyToBoard,
  };
});
