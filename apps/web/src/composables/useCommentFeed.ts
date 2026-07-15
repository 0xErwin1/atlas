import { computed, ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import { wrappedClient } from '@/api/wrapper';
import { errorHint } from '@/lib/apiError';

type CommentListResponse = components['schemas']['CommentListResponseDto'];
type CommentFeedPage = components['schemas']['Page_CommentFeedEntryDto'];
type CommentFeedEntry = CommentFeedPage['items'][number];
type CommentLinkTarget = components['schemas']['CommentLinkTargetDto'];

export type CommentParentTarget =
  | { kind: 'task'; ws: string; readableId: string }
  | { kind: 'document'; ws: string; slug: string };

export type AvailableCommentLinkTarget = Extract<CommentLinkTarget, { status: 'available' }>;
export type UnavailableCommentLinkTarget = { status: 'unavailable'; label: 'Recurso no disponible' };
export type NormalizedCommentLinkTarget = AvailableCommentLinkTarget | UnavailableCommentLinkTarget;

type CommentFeedEvent = Extract<CommentFeedEntry, { type: 'event' }>;
type NormalizedCommentFeedEvent = Omit<CommentFeedEvent, 'target'> & {
  target?: NormalizedCommentLinkTarget | null;
};

export type NormalizedCommentFeedEntry =
  | {
      type: 'comment';
      comment: components['schemas']['CommentDto'];
      links: Array<{ target: NormalizedCommentLinkTarget }>;
    }
  | NormalizedCommentFeedEvent;

type FeedStatus = 'idle' | 'pending' | 'ready' | 'error';

function sameTarget(left: CommentParentTarget | null, right: CommentParentTarget): boolean {
  if (left?.kind !== right.kind || left.ws !== right.ws) return false;

  return left.kind === 'task'
    ? left.readableId === (right.kind === 'task' ? right.readableId : '')
    : left.slug === (right.kind === 'document' ? right.slug : '');
}

function isFullCommentFeed(data: CommentListResponse): data is CommentFeedPage {
  return data.items.every((item) => 'type' in item);
}

function entryId(entry: NormalizedCommentFeedEntry): string {
  return entry.type === 'comment' ? `comment:${entry.comment.id}` : `event:${entry.id}`;
}

function normalizeFeedEntries(entries: CommentFeedEntry[]): NormalizedCommentFeedEntry[] {
  return entries.map((entry) =>
    entry.type === 'comment'
      ? { ...entry, links: entry.links.map((link) => ({ target: normalizeCommentLinkTarget(link.target) })) }
      : normalizeCommentFeedEvent(entry),
  );
}

function normalizeCommentFeedEvent(entry: CommentFeedEvent): NormalizedCommentFeedEvent {
  const { target, ...event } = entry;

  if (target === undefined) return event;
  if (target === null) return { ...event, target: null };

  return { ...event, target: normalizeCommentLinkTarget(target) };
}

export function normalizeCommentLinkTarget(target: CommentLinkTarget): NormalizedCommentLinkTarget {
  if (target.status === 'unavailable') {
    return { status: 'unavailable', label: 'Recurso no disponible' };
  }

  return target;
}

export function useCommentFeed() {
  const entries = ref<NormalizedCommentFeedEntry[]>([]);
  const cursor = ref<string | null>(null);
  const hasMore = ref(false);
  const status = ref<FeedStatus>('idle');
  const error = ref<string | null>(null);
  const target = ref<CommentParentTarget | null>(null);
  let generation = 0;
  let requestSequence = 0;

  const isLoading = computed(() => status.value === 'pending');

  function isCurrent(
    requestTarget: CommentParentTarget,
    requestGeneration: number,
    sequence: number,
  ): boolean {
    return (
      generation === requestGeneration &&
      requestSequence === sequence &&
      sameTarget(target.value, requestTarget)
    );
  }

  function reset(nextTarget: CommentParentTarget): number {
    if (!sameTarget(target.value, nextTarget)) {
      generation += 1;
      target.value = nextTarget;
      entries.value = [];
      cursor.value = null;
      hasMore.value = false;
    }

    return generation;
  }

  async function requestFeed(
    requestTarget: CommentParentTarget,
    requestCursor?: string,
  ): Promise<{ data?: CommentListResponse; error?: unknown }> {
    const query = {
      feed: 'full' as const,
      ...(requestCursor === undefined ? {} : { cursor: requestCursor }),
    };

    if (requestTarget.kind === 'task') {
      return wrappedClient.GET('/api/workspaces/{ws}/tasks/{readable_id}/comments', {
        params: { path: { ws: requestTarget.ws, readable_id: requestTarget.readableId }, query },
      });
    }

    return wrappedClient.GET('/api/workspaces/{ws}/documents/{slug}/comments', {
      params: { path: { ws: requestTarget.ws, slug: requestTarget.slug }, query },
    });
  }

  function applyPage(page: CommentFeedPage, append: boolean): void {
    const incoming = normalizeFeedEntries(page.items);
    const merged = append ? [...entries.value, ...incoming] : incoming;
    const ids = new Set<string>();
    entries.value = merged.filter((entry) => {
      const id = entryId(entry);
      if (ids.has(id)) return false;
      ids.add(id);
      return true;
    });
    cursor.value = page.next_cursor ?? null;
    hasMore.value = page.has_more;
  }

  async function load(nextTarget: CommentParentTarget): Promise<void> {
    const requestGeneration = reset(nextTarget);
    const sequence = ++requestSequence;
    status.value = 'pending';
    error.value = null;

    try {
      const { data, error: apiError } = await requestFeed(nextTarget);
      if (!isCurrent(nextTarget, requestGeneration, sequence)) return;

      if (apiError !== undefined || data === undefined || !isFullCommentFeed(data)) {
        entries.value = [];
        cursor.value = null;
        hasMore.value = false;
        status.value = 'error';
        error.value = errorHint(apiError, 'Failed to load comments');
        return;
      }

      applyPage(data, false);
      status.value = 'ready';
    } catch (cause) {
      if (!isCurrent(nextTarget, requestGeneration, sequence)) return;

      entries.value = [];
      cursor.value = null;
      hasMore.value = false;
      status.value = 'error';
      error.value = errorHint(cause, 'Failed to load comments');
    }
  }

  async function loadMore(requestTarget: CommentParentTarget): Promise<void> {
    if (!sameTarget(target.value, requestTarget) || !hasMore.value || cursor.value === null) return;

    const requestGeneration = generation;
    const sequence = ++requestSequence;
    const requestCursor = cursor.value;
    status.value = 'pending';
    error.value = null;

    try {
      const { data, error: apiError } = await requestFeed(requestTarget, requestCursor);
      if (!isCurrent(requestTarget, requestGeneration, sequence)) return;

      if (apiError !== undefined || data === undefined || !isFullCommentFeed(data)) {
        status.value = 'error';
        error.value = errorHint(apiError, 'Failed to load comments');
        return;
      }

      applyPage(data, true);
      status.value = 'ready';
    } catch (cause) {
      if (!isCurrent(requestTarget, requestGeneration, sequence)) return;

      status.value = 'error';
      error.value = errorHint(cause, 'Failed to load comments');
    }
  }

  return {
    entries,
    cursor,
    hasMore,
    status,
    error,
    isLoading,
    load,
    loadMore,
  };
}
