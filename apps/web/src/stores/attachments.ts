import { defineStore } from 'pinia';
import { ref } from 'vue';
import type { components } from '@/api/types';
import { wrappedClient } from '@/api/wrapper';
import { errorHint } from '@/lib/apiError';

export type WorkspaceAttachment = components['schemas']['WorkspaceAttachmentDto'];
export type AttachmentOwner = components['schemas']['AttachmentOwnerDto'];

/** Owner kinds the listing can be narrowed to; `all` lifts the restriction. */
export type OwnerFilter = 'all' | 'document' | 'task';

/** Coarse content-type buckets the filter bar offers. */
export type TypeFilter = 'all' | 'image' | 'other';

export interface AttachmentFilter {
  query: string;
  owner: OwnerFilter;
  type: TypeFilter;
}

export const DEFAULT_ATTACHMENT_FILTER: AttachmentFilter = { query: '', owner: 'all', type: 'all' };

const PAGE_SIZE = 50;

/**
 * The `other` bucket has no single prefix, so it is applied client-side over a
 * page the server already narrowed by name and owner.
 */
function matchesType(attachment: WorkspaceAttachment, type: TypeFilter): boolean {
  if (type === 'all') return true;
  const isImage = attachment.content_type.startsWith('image/');
  return type === 'image' ? isImage : !isImage;
}

export const useAttachmentsStore = defineStore('attachments', () => {
  const items = ref<WorkspaceAttachment[]>([]);
  const nextCursor = ref<string | null>(null);
  const hasMore = ref(false);
  const loading = ref(false);
  const error = ref<string | null>(null);
  const filter = ref<AttachmentFilter>({ ...DEFAULT_ATTACHMENT_FILTER });
  let requestGeneration = 0;

  async function load(
    ws: string,
    nextFilter: AttachmentFilter = DEFAULT_ATTACHMENT_FILTER,
    cursor?: string,
  ): Promise<void> {
    if (ws === '') return;

    const generation = ++requestGeneration;
    const replace = cursor === undefined;

    filter.value = nextFilter;
    if (replace) {
      items.value = [];
      nextCursor.value = null;
      hasMore.value = false;
    }

    loading.value = true;
    error.value = null;

    const trimmed = nextFilter.query.trim();
    const query = {
      limit: PAGE_SIZE,
      ...(trimmed === '' ? {} : { q: trimmed }),
      ...(nextFilter.owner === 'all' ? {} : { owner: nextFilter.owner }),
      ...(nextFilter.type === 'image' ? { content_type: 'image/' } : {}),
      ...(cursor === undefined ? {} : { cursor }),
    };

    const { data, error: apiError } = await wrappedClient.GET('/api/workspaces/{ws}/attachments', {
      params: { path: { ws }, query },
    });

    if (generation !== requestGeneration) return;

    loading.value = false;
    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to load files');
      return;
    }

    const page = data.items.filter((item) => matchesType(item, nextFilter.type));
    items.value = replace ? page : [...items.value, ...page];
    nextCursor.value = data.next_cursor ?? null;
    hasMore.value = data.has_more;
  }

  async function loadMore(ws: string): Promise<void> {
    if (!hasMore.value || nextCursor.value === null || loading.value) return;
    await load(ws, filter.value, nextCursor.value);
  }

  /** Renames the file and rewrites the `[[file:…]]` links that addressed it. */
  async function rename(ws: string, attachmentId: string, fileName: string): Promise<boolean> {
    error.value = null;

    const { data, error: apiError } = await wrappedClient.PATCH(
      '/api/workspaces/{ws}/attachments/{attachment_id}',
      {
        params: { path: { ws, attachment_id: attachmentId } },
        body: { file_name: fileName },
      },
    );

    if (apiError !== undefined || data === undefined) {
      error.value = errorHint(apiError, 'Failed to rename file');
      return false;
    }

    items.value = items.value.map((item) => (item.id === attachmentId ? data : item));
    return true;
  }

  async function remove(ws: string, attachmentId: string): Promise<boolean> {
    error.value = null;

    const { error: apiError } = await wrappedClient.DELETE(
      '/api/workspaces/{ws}/attachments/{attachment_id}',
      { params: { path: { ws, attachment_id: attachmentId } } },
    );

    if (apiError !== undefined) {
      error.value = errorHint(apiError, 'Failed to delete file');
      return false;
    }

    items.value = items.value.filter((item) => item.id !== attachmentId);
    return true;
  }

  return {
    items,
    nextCursor,
    hasMore,
    loading,
    error,
    filter,
    load,
    loadMore,
    rename,
    remove,
  };
});
