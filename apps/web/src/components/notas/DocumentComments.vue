<script setup lang="ts">
/**
 * Document comment thread: a pure comments panel (no activity feed) for the open
 * note. Loads the thread on mount and whenever the slug changes, renders the
 * comments oldest-first with a pinned composer, and mirrors the task thread's
 * author/moderator gating: editing is author-only, deletion is author-or-moderator.
 */
import { computed, watch } from 'vue';
import CommentCard from '@/components/comments/CommentCard.vue';
import CommentComposer from '@/components/comments/CommentComposer.vue';
import EmptyState from '@/components/states/EmptyState.vue';
import ErrorState from '@/components/states/ErrorState.vue';
import LoadingState from '@/components/states/LoadingState.vue';
import { useAuthStore } from '@/stores/auth';
import { type CommentDto, useDocumentsStore } from '@/stores/documents';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

const props = defineProps<{
  ws: string;
  slug: string;
}>();

const documents = useDocumentsStore();
const ui = useUiStore();
const auth = useAuthStore();
const workspace = useWorkspaceStore();

const currentActorId = computed(() => auth.user?.id ?? null);

// The server authorizes deletion (author OR workspace admin/owner); this only
// gates the affordance, exactly as the task comment thread does.
const canModerate = computed(
  () => workspace.myWorkspaceRole === 'owner' || workspace.myWorkspaceRole === 'admin',
);

// Editing is author-only: the server forbids admins from editing others' comments.
function canEdit(comment: CommentDto): boolean {
  return currentActorId.value !== null && comment.author.id === currentActorId.value;
}

function canDelete(comment: CommentDto): boolean {
  if (canModerate.value) return true;
  return currentActorId.value !== null && comment.author.id === currentActorId.value;
}

async function onSubmit(body: string): Promise<boolean> {
  const ok = await documents.addComment(props.ws, props.slug, body);
  if (!ok && documents.error) ui.showBanner(documents.error, 'error');
  return ok;
}

async function onSave(id: string, body: string): Promise<boolean> {
  const ok = await documents.editComment(props.ws, props.slug, id, body);
  if (!ok && documents.error) ui.showBanner(documents.error, 'error');
  return ok;
}

async function onDelete(id: string): Promise<void> {
  const ok = await documents.removeComment(props.ws, props.slug, id);
  if (!ok && documents.error) ui.showBanner(documents.error, 'error');
}

async function loadMore(): Promise<void> {
  await documents.loadMoreComments(props.ws, props.slug);
  if (documents.commentsError) ui.showBanner(documents.commentsError, 'error');
}

watch(
  () => [props.ws, props.slug] as const,
  ([ws, slug]) => {
    void documents.loadComments(ws, slug);
  },
  { immediate: true },
);
</script>

<template>
  <section class="atl-ac">
    <LoadingState
      v-if="documents.commentsStatus === 'pending' && documents.comments.length === 0"
      label="Loading comments…"
    />

    <ErrorState
      v-else-if="documents.commentsStatus === 'error'"
      title="Could not load comments"
      :hint="documents.commentsError ?? undefined"
      @retry="documents.loadComments(ws, slug)"
    />

    <EmptyState
      v-else-if="documents.comments.length === 0"
      compact
      icon="message-square"
      title="No comments yet"
      hint="Start the conversation — comments support markdown."
    />

    <div v-else class="flex flex-col" style="gap: 12px;">
      <CommentCard
        v-for="comment in documents.comments"
        :key="comment.id"
        :comment="comment"
        :can-edit="canEdit(comment)"
        :can-delete="canDelete(comment)"
        :on-save="onSave"
        :on-delete="onDelete"
      />
    </div>

    <div v-if="documents.commentsStatus !== 'error' && documents.commentsHasMore" style="margin-top: 12px;">
      <button
        type="button"
        data-test="comment-load-more"
        class="atl-comment-btn"
        @click="loadMore"
      >
        Load more comments
      </button>
    </div>

    <CommentComposer v-if="documents.commentsStatus !== 'error'" :on-submit="onSubmit" />
  </section>
</template>

<style scoped>
.atl-ac {
  display: flex;
  flex-direction: column;
}
</style>
