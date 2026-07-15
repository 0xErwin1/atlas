<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue';
import { type RouteLocationRaw, RouterLink, useRouter } from 'vue-router';
import CommentCard from '@/components/comments/CommentCard.vue';
import CommentComposer from '@/components/comments/CommentComposer.vue';
import EmptyState from '@/components/states/EmptyState.vue';
import AgentBadge from '@/components/ui/AgentBadge.vue';
import Avatar from '@/components/ui/Avatar.vue';
import {
  type AvailableCommentLinkTarget,
  type CommentParentTarget,
  type NormalizedCommentFeedEntry,
  useCommentFeed,
} from '@/composables/useCommentFeed';
import { activityVerb } from '@/lib/activityVerb';
import { actorName, isAgent } from '@/lib/actor';
import { formatDate } from '@/lib/format';
import { useAuthStore } from '@/stores/auth';
import { type ActivityEntryDto, type CommentDto, useTaskDetailStore } from '@/stores/taskDetail';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

const props = withDefaults(
  defineProps<{
    ws: string;
    readableId: string;
    /** Fill the host height with the composer pinned to the bottom and only the
     * feed scrolling (desktop dock/modal/inspector). Off for the inline mobile
     * feed, which grows within the page scroll. */
    pinned?: boolean;
  }>(),
  { pinned: false },
);

const detail = useTaskDetailStore();
const ui = useUiStore();
const auth = useAuthStore();
const workspace = useWorkspaceStore();
const router = useRouter();
const commentFeed = useCommentFeed();
const commentEntries = commentFeed.entries;
const commentHasMore = commentFeed.hasMore;
const commentStatus = commentFeed.status;
const commentError = commentFeed.error;
const commentAttachments = commentFeed.attachments;
const commentAttachmentError = commentFeed.attachmentError;
const commentTarget = computed<CommentParentTarget>(() => ({
  kind: 'task',
  ws: props.ws,
  readableId: props.readableId,
}));
const attachmentCommentsLoaded = new Set<string>();

/** A navigable target surfaced next to a reference/mention activity entry. */
interface ActivityLink {
  label: string;
  to: RouteLocationRaw;
}

// One chronological feed: system activity entries and user comments interleaved
// by time (oldest first), with the composer pinned below. ISO timestamps sort
// lexicographically, so a string compare is chronological.
type FeedItem =
  | {
      kind: 'comment';
      key: string;
      at: string;
      comment: CommentDto;
      links: Extract<NormalizedCommentFeedEntry, { type: 'comment' }>['links'];
    }
  | {
      kind: 'comment-event';
      key: string;
      at: string;
      event: Extract<NormalizedCommentFeedEntry, { type: 'event' }>;
    }
  | { kind: 'activity'; key: string; at: string; entry: ActivityEntryDto; link: ActivityLink | null };

function asRecord(value: unknown): Record<string, unknown> | null {
  return typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : null;
}

/**
 * The task or document an activity entry points at, so the feed both names it and
 * links to it. `reference_added` carries only the reference id, resolved against
 * the task's loaded references (a later-removed reference degrades to no link);
 * `document_mentioned` carries the target document inline.
 */
function activityLink(entry: ActivityEntryDto): ActivityLink | null {
  const payload = asRecord(entry.payload);
  if (payload === null) return null;

  if (entry.kind === 'reference_added') {
    const body = asRecord(payload.reference_added);
    const referenceId = typeof body?.reference_id === 'string' ? body.reference_id : null;
    if (referenceId === null) return null;

    const ref = detail.references.find((r) => r.id === referenceId);
    if (ref === undefined || !ref.target_resolved) return null;

    if (ref.target_readable_id != null) {
      return {
        label: ref.target_readable_id,
        to: { name: 'task-detail', params: { readableId: ref.target_readable_id } },
      };
    }
    if (ref.target_document_id != null) {
      return {
        label: ref.target_title ?? 'document',
        to: { name: 'notes', params: { slug: ref.target_document_id } },
      };
    }
    return null;
  }

  if (entry.kind === 'document_mentioned') {
    const body = asRecord(payload.document_mentioned);
    const documentId = typeof body?.document_id === 'string' ? body.document_id : null;
    if (documentId === null) return null;

    const title = typeof body?.title === 'string' ? body.title : 'document';
    return { label: title, to: { name: 'notes', params: { slug: documentId } } };
  }

  return null;
}

const feed = computed<FeedItem[]>(() => {
  const items: FeedItem[] = [
    ...commentFeed.entries.value.map(
      (entry): FeedItem =>
        entry.type === 'comment'
          ? {
              kind: 'comment',
              key: `c:${entry.comment.id}`,
              at: entry.comment.created_at,
              comment: entry.comment,
              links: entry.links,
            }
          : { kind: 'comment-event', key: `e:${entry.id}`, at: entry.created_at, event: entry },
    ),
    ...detail.activity.map(
      (entry): FeedItem => ({
        kind: 'activity',
        key: `a:${entry.id}`,
        at: entry.created_at,
        entry,
        link: activityLink(entry),
      }),
    ),
  ];

  return items.sort((a, b) => (a.at < b.at ? -1 : a.at > b.at ? 1 : 0));
});

const isEmpty = computed(() => feed.value.length === 0 && commentStatus.value !== 'pending');

// The scrollable feed viewport (pinned mode only). Entering a task lands at the
// newest entry, and posting a comment follows the thread down; a user who has
// scrolled up to read history is left where they are.
const scrollRef = ref<HTMLElement | null>(null);

// Set when the open task changes so the *next* feed population scrolls to the
// end, regardless of whether the collections load before or after mount.
let scrollToEndPending = props.pinned;

function scrollToBottom(): void {
  const el = scrollRef.value;
  if (el !== null) el.scrollTop = el.scrollHeight;
}

function isNearBottom(): boolean {
  const el = scrollRef.value;
  if (el === null) return true;
  return el.scrollHeight - el.scrollTop - el.clientHeight < 48;
}

onMounted(() => {
  if (props.pinned && !isEmpty.value) void nextTick(scrollToBottom);
});

watch(
  () => props.readableId,
  () => {
    if (props.pinned) scrollToEndPending = true;
  },
);

// React to the feed being (re)populated: the initial load and every task switch
// resolve their collections asynchronously, so the jump to the end happens here
// once the items actually exist.
watch(
  () => feed.value.length,
  (next, prev) => {
    if (!props.pinned || next === 0) return;

    if (scrollToEndPending) {
      scrollToEndPending = false;
      void nextTick(scrollToBottom);
    } else if (next > prev && isNearBottom()) {
      void nextTick(scrollToBottom);
    }
  },
);

// The server authorizes deletion (author OR workspace admin/owner); this only
// gates whether the affordance is shown. A break-glass global admin with no
// membership row here sees no button and would get a 403, which is acceptable.
const canModerate = computed(
  () => workspace.myWorkspaceRole === 'owner' || workspace.myWorkspaceRole === 'admin',
);

const currentActorId = computed(() => auth.user?.id ?? null);

// Editing is author-only: the server forbids admins from editing others' comments.
function canEdit(comment: CommentDto): boolean {
  return currentActorId.value !== null && comment.author.id === currentActorId.value;
}

function canDelete(comment: CommentDto): boolean {
  if (canModerate.value) return true;
  return currentActorId.value !== null && comment.author.id === currentActorId.value;
}

async function onSubmit(body: string): Promise<boolean> {
  const ok = await detail.addComment(props.ws, props.readableId, body);
  if (!ok && detail.error) ui.showBanner(detail.error, 'error');
  // Follow the user's own comment down to the bottom of the thread.
  if (ok) {
    await commentFeed.load(commentTarget.value);
    if (props.pinned) void nextTick(scrollToBottom);
  }
  return ok;
}

async function onSave(id: string, body: string): Promise<boolean> {
  const ok = await detail.editComment(props.ws, props.readableId, id, body);
  if (!ok && detail.error) ui.showBanner(detail.error, 'error');
  if (ok) await commentFeed.load(commentTarget.value);
  return ok;
}

async function onDelete(id: string): Promise<void> {
  const ok = await detail.removeComment(props.ws, props.readableId, id);
  if (!ok && detail.error) ui.showBanner(detail.error, 'error');
  if (ok) await commentFeed.load(commentTarget.value);
}

async function loadMore(): Promise<void> {
  await commentFeed.loadMore(commentTarget.value);
  if (commentFeed.error.value !== null) ui.showBanner(commentFeed.error.value, 'error');
}

async function navigateCommentTarget(target: AvailableCommentLinkTarget, commentId?: string): Promise<void> {
  if (target.type === 'task') void router.push({ name: 'task-detail', params: { readableId: target.id } });
  else if (target.type === 'document') void router.push({ name: 'notes', params: { slug: target.id } });
  else if (target.type === 'attachment' && commentId !== undefined) {
    await downloadCommentAttachment(commentId, target.id);
  }
}

function attachmentUrl(commentId: string, attachmentId: string): string {
  return `/api/workspaces/${props.ws}/tasks/${props.readableId}/comments/${commentId}/attachments/${attachmentId}/content`;
}

async function uploadCommentAttachment(commentId: string, file: File) {
  return commentFeed.uploadAttachment(commentTarget.value, commentId, file);
}

async function downloadCommentAttachment(commentId: string, attachmentId: string): Promise<Blob | null> {
  const blob = await commentFeed.downloadAttachment(commentTarget.value, commentId, attachmentId);
  if (blob === null) return null;

  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download =
    commentFeed.attachments.value[commentId]?.find((item) => item.id === attachmentId)?.file_name ??
    'attachment';
  anchor.click();
  URL.revokeObjectURL(url);
  return blob;
}

async function deleteCommentAttachment(commentId: string, attachmentId: string): Promise<boolean> {
  return commentFeed.deleteAttachment(commentTarget.value, commentId, attachmentId);
}

watch(
  commentTarget,
  (target) => {
    attachmentCommentsLoaded.clear();
    void commentFeed.load(target);
  },
  { immediate: true },
);

watch(
  () => commentFeed.entries.value,
  (entries) => {
    for (const entry of entries) {
      if (entry.type !== 'comment' || attachmentCommentsLoaded.has(entry.comment.id)) continue;

      attachmentCommentsLoaded.add(entry.comment.id);
      void commentFeed.loadAttachments(commentTarget.value, entry.comment.id);
    }
  },
  { deep: true, immediate: true },
);
</script>

<template>
  <section class="atl-ac" :class="{ pinned }">
    <div ref="scrollRef" class="atl-ac-scroll">
      <div v-if="commentStatus === 'pending' && feed.length === 0" role="status">Loading comments…</div>
      <div v-else-if="commentStatus === 'error'" role="alert">
        {{ commentError ?? 'Could not load comments' }}
        <button type="button" class="atl-comment-btn" @click="commentFeed.load(commentTarget)">Retry</button>
      </div>

      <EmptyState
        v-if="isEmpty"
        compact
        icon="message-square"
        title="No activity yet"
        hint="Activity and comments show up here — comments support markdown."
      />

      <div v-else class="atl-ac-feed flex flex-col" style="gap: 12px;">
        <template v-for="item in feed" :key="item.key">
        <div
          v-if="item.kind === 'activity'"
          class="flex items-start"
          style="gap: 8px;"
          :data-activity-id="item.entry.id"
        >
          <Avatar
            :name="actorName(item.entry.actor.display_name, item.entry.actor.type)"
            :agent="isAgent(item.entry.actor.type)"
            :size="20"
          />
          <div class="flex flex-col" style="gap: 2px; min-width: 0;">
            <div class="flex items-center" style="gap: 6px; flex-wrap: wrap;">
              <span
                style="font-family: var(--font-mono); font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--c-foreground);"
              >
                {{ actorName(item.entry.actor.display_name, item.entry.actor.type) }}
              </span>
              <AgentBadge v-if="isAgent(item.entry.actor.type)" />
              <span style="font-size: var(--fs-sm); color: var(--c-muted);">{{ activityVerb(item.entry.kind) }}</span>
              <template v-if="item.link">
                <span aria-hidden="true" style="font-size: var(--fs-sm); color: var(--c-muted);">→</span>
                <RouterLink :to="item.link.to" class="atl-ac-reflink">{{ item.link.label }}</RouterLink>
              </template>
            </div>
            <span style="font-size: var(--fs-xs); color: var(--c-muted);">
              {{ formatDate(item.entry.created_at) }}
            </span>
          </div>
        </div>

        <CommentCard
            v-else-if="item.kind === 'comment'"
            :comment="item.comment"
            :can-edit="canEdit(item.comment)"
            :can-delete="canDelete(item.comment)"
            :on-save="onSave"
            :on-delete="onDelete"
            :links="item.links"
            :attachments="commentAttachments[item.comment.id] ?? []"
            :can-manage-attachments="canDelete(item.comment)"
            :attachment-uploading="commentFeed.isAttachmentUploadLoading(item.comment.id)"
            :attachment-error="commentAttachmentError[item.comment.id]"
            :on-upload-attachment="(file) => uploadCommentAttachment(item.comment.id, file)"
            :on-download-attachment="(attachmentId) => downloadCommentAttachment(item.comment.id, attachmentId)"
            :on-delete-attachment="(attachmentId) => deleteCommentAttachment(item.comment.id, attachmentId)"
            :upload-image="async (file) => {
              const attachment = await uploadCommentAttachment(item.comment.id, file);
              return attachment === null ? null : attachmentUrl(item.comment.id, attachment.id);
            }"
            @navigate-link="navigateCommentTarget"
          />
        <CommentCard v-else :event="item.event" @navigate-link="navigateCommentTarget" />
        </template>
      </div>

      <div v-if="commentHasMore" style="margin-top: 12px;">
        <button
          type="button"
          data-test="comment-load-more"
          class="atl-comment-btn"
          @click="loadMore"
        >
          Load more comments
        </button>
      </div>
    </div>

    <div class="atl-ac-composer">
      <CommentComposer :on-submit="onSubmit" />
    </div>
  </section>
</template>

<style scoped>
.atl-ac {
  display: flex;
  flex-direction: column;
}

.atl-ac-reflink {
  min-width: 0;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: var(--font-mono);
  font-size: var(--fs-sm);
  color: var(--c-primary);
  cursor: pointer;
}

.atl-ac-reflink:hover {
  text-decoration: underline;
}

/* Inline (mobile) feed: grows within the page scroll, composer trailing it. */
.atl-ac:not(.pinned) .atl-ac-composer {
  margin-top: 16px;
}

/* Pinned (dock/modal/inspector): fill the host height so only the feed scrolls
   and the composer stays docked at the bottom. */
.atl-ac.pinned {
  flex: 1;
  min-height: 0;
  height: 100%;
}

.atl-ac.pinned .atl-ac-scroll {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 12px 14px;
}

.atl-ac.pinned .atl-ac-composer {
  flex: 0 0 auto;
  padding: 10px 14px;
  border-top: 1px solid var(--c-border);
  background: var(--c-panel);
}
</style>
