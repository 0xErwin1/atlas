import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types';
import { wrappedClient } from '@/api/wrapper';
import { errorHint } from '@/lib/apiError';

export type TrashItem = components['schemas']['TrashItemDto'];
export type TrashKind = components['schemas']['TrashKindDto'];
export type PurgeStatus = components['schemas']['PurgeStatusDtoResponse'];

export interface TrashFilter {
  workspaceId?: string;
  kind?: TrashKind;
}

const PAGE_SIZE = 50;

export const useTrashStore = defineStore('trash', () => {
  const items = ref<TrashItem[]>([]);
  const nextCursor = ref<string | null>(null);
  const hasMore = ref(false);
  const loading = ref(false);
  const error = ref<string | null>(null);
  const filter = ref<TrashFilter>({});

  async function load(nextFilter: TrashFilter = {}, cursor?: string): Promise<void> {
    loading.value = true;
    error.value = null;

    const query = {
      limit: PAGE_SIZE,
      ...(nextFilter.workspaceId !== undefined ? { workspace_id: nextFilter.workspaceId } : {}),
      ...(nextFilter.kind !== undefined ? { kind: nextFilter.kind } : {}),
      ...(cursor !== undefined ? { cursor } : {}),
    };
    const { data, error: apiError } = await wrappedClient.GET('/api/admin/trash', { params: { query } });

    loading.value = false;
    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to load Trash');
      return;
    }

    filter.value = nextFilter;
    items.value = cursor === undefined ? data.items : [...items.value, ...data.items];
    nextCursor.value = data.next_cursor ?? null;
    hasMore.value = data.has_more;
  }

  async function loadMore(): Promise<void> {
    if (!hasMore.value || nextCursor.value === null || loading.value) return;
    await load(filter.value, nextCursor.value);
  }

  async function restore(item: TrashItem): Promise<boolean> {
    error.value = null;
    const { error: apiError } = await wrappedClient.POST('/api/admin/trash/restore', {
      body: { kind: item.kind, target_id: item.target_id },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to restore item');
      return false;
    }

    await load(filter.value);
    return error.value === null;
  }

  async function purge(item: TrashItem): Promise<PurgeStatus | null> {
    error.value = null;
    const { data, error: apiError } = await wrappedClient.POST('/api/admin/trash/purge', {
      body: { kind: item.kind, target_id: item.target_id, confirm: true },
    });

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to purge item');
      return null;
    }

    if (data === undefined) {
      await load(filter.value);
      return null;
    }

    return data;
  }

  async function poll(operationId: string): Promise<PurgeStatus | null> {
    if (operationId === '') return null;

    const { data, error: apiError } = await wrappedClient.GET('/api/admin/trash/purges/{operation_id}', {
      params: { path: { operation_id: operationId } },
    });

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to check purge status');
      return null;
    }

    if (data.status === 'complete') await load(filter.value);
    return data;
  }

  return { items, nextCursor, hasMore, loading, error, filter, load, loadMore, restore, purge, poll };
});
