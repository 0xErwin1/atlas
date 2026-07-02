<script setup lang="ts">
import { computed } from 'vue';
import CommentCard from '@/components/comments/CommentCard.vue';
import CommentComposer from '@/components/comments/CommentComposer.vue';
import EmptyState from '@/components/states/EmptyState.vue';
import AgentBadge from '@/components/ui/AgentBadge.vue';
import Avatar from '@/components/ui/Avatar.vue';
import { activityVerb } from '@/lib/activityVerb';
import { actorName, isAgent } from '@/lib/actor';
import { formatDate } from '@/lib/format';
import { useAuthStore } from '@/stores/auth';
import { type ActivityEntryDto, type CommentDto, useTaskDetailStore } from '@/stores/taskDetail';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

const props = defineProps<{
  ws: string;
  readableId: string;
}>();

const detail = useTaskDetailStore();
const ui = useUiStore();
const auth = useAuthStore();
const workspace = useWorkspaceStore();

// One chronological feed: system activity entries and user comments interleaved
// by time (oldest first), with the composer pinned below. ISO timestamps sort
// lexicographically, so a string compare is chronological.
type FeedItem =
  | { kind: 'comment'; key: string; at: string; comment: CommentDto }
  | { kind: 'activity'; key: string; at: string; entry: ActivityEntryDto };

const feed = computed<FeedItem[]>(() => {
  const items: FeedItem[] = [
    ...detail.comments.map(
      (comment): FeedItem => ({ kind: 'comment', key: `c:${comment.id}`, at: comment.created_at, comment }),
    ),
    ...detail.activity.map(
      (entry): FeedItem => ({ kind: 'activity', key: `a:${entry.id}`, at: entry.created_at, entry }),
    ),
  ];

  return items.sort((a, b) => (a.at < b.at ? -1 : a.at > b.at ? 1 : 0));
});

const isEmpty = computed(() => detail.comments.length === 0 && detail.activity.length === 0);

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
  return ok;
}

async function onSave(id: string, body: string): Promise<boolean> {
  const ok = await detail.editComment(props.ws, props.readableId, id, body);
  if (!ok && detail.error) ui.showBanner(detail.error, 'error');
  return ok;
}

async function onDelete(id: string): Promise<void> {
  const ok = await detail.removeComment(props.ws, props.readableId, id);
  if (!ok && detail.error) ui.showBanner(detail.error, 'error');
}

async function loadMore(): Promise<void> {
  await detail.loadMoreComments(props.ws, props.readableId);
  if (detail.error) ui.showBanner(detail.error, 'error');
}
</script>

<template>
  <section class="atl-ac">
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
            </div>
            <span style="font-size: var(--fs-xs); color: var(--c-muted);">
              {{ formatDate(item.entry.created_at) }}
            </span>
          </div>
        </div>

        <CommentCard
          v-else
          :comment="item.comment"
          :can-edit="canEdit(item.comment)"
          :can-delete="canDelete(item.comment)"
          :on-save="onSave"
          :on-delete="onDelete"
        />
      </template>
    </div>

    <div v-if="detail.commentsHasMore" style="margin-top: 12px;">
      <button
        type="button"
        data-test="comment-load-more"
        class="atl-comment-btn"
        @click="loadMore"
      >
        Load more comments
      </button>
    </div>

    <CommentComposer :on-submit="onSubmit" />
  </section>
</template>

<style scoped>
.atl-ac {
  display: flex;
  flex-direction: column;
}
</style>
