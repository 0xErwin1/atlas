<script setup lang="ts">
/**
 * Document comment thread: a pure comments panel (no activity feed) for the open
 * note. Loads the thread on mount and whenever the slug changes, renders the
 * comments oldest-first with a pinned composer, and mirrors the task thread's
 * author/moderator gating: editing is author-only, deletion is author-or-moderator.
 */
import { computed, watch } from 'vue';
import { useRouter } from 'vue-router';
import CommentCard from '@/components/comments/CommentCard.vue';
import CommentComposer from '@/components/comments/CommentComposer.vue';
import EmptyState from '@/components/states/EmptyState.vue';
import ErrorState from '@/components/states/ErrorState.vue';
import LoadingState from '@/components/states/LoadingState.vue';
import { useCommentAttachments } from '@/composables/useCommentAttachments';
import {
  type AvailableCommentLinkTarget,
  type CommentParentTarget,
  useCommentFeed,
} from '@/composables/useCommentFeed';
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
const router = useRouter();
const commentFeed = useCommentFeed();
const commentEntries = commentFeed.entries;
const commentHasMore = commentFeed.hasMore;
const commentStatus = commentFeed.status;
const commentError = commentFeed.error;
const commentTarget = computed<CommentParentTarget>(() => ({
  kind: 'document',
  ws: props.ws,
  slug: props.slug,
}));
const {
  items: commentAttachmentItems,
  error: commentAttachmentError,
  isListing: isAttachmentListing,
  isUploading: isAttachmentUploading,
  isDownloading: isAttachmentDownloading,
  isDeleting: isAttachmentDeleting,
  reload: reloadCommentAttachments,
  upload: uploadCommentAttachment,
  download: downloadCommentAttachment,
  delete: deleteCommentAttachment,
  contentUrl: attachmentContentUrl,
} = useCommentAttachments(commentTarget, commentEntries);

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

async function onSubmit(body: string, draftId?: string): Promise<boolean> {
  const ok =
    draftId === undefined
      ? await documents.addComment(props.ws, props.slug, body)
      : await documents.addComment(props.ws, props.slug, body, draftId);
  if (!ok && documents.error) ui.showBanner(documents.error, 'error');
  if (ok) await commentFeed.load(commentTarget.value);
  return ok;
}

async function onSave(id: string, body: string): Promise<boolean> {
  const ok = await documents.editComment(props.ws, props.slug, id, body);
  if (!ok && documents.error) ui.showBanner(documents.error, 'error');
  if (ok) await commentFeed.load(commentTarget.value);
  return ok;
}

async function onDelete(id: string): Promise<void> {
  const ok = await documents.removeComment(props.ws, props.slug, id);
  if (!ok && documents.error) ui.showBanner(documents.error, 'error');
  if (ok) await commentFeed.load(commentTarget.value);
}

async function uploadImage(commentId: string, file: File): Promise<string | null> {
  const attachment = await uploadCommentAttachment(commentId, file);
  return attachment === null ? null : attachmentContentUrl(commentId, attachment.id);
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

watch(
  commentTarget,
  (target) => {
    void commentFeed.load(target);
  },
  { immediate: true },
);
</script>

<template>
  <section class="atl-ac">
    <LoadingState
      v-if="commentStatus === 'pending' && commentEntries.length === 0"
      label="Loading comments…"
    />

    <ErrorState
      v-else-if="commentStatus === 'error'"
      title="Could not load comments"
      :hint="commentError ?? undefined"
      @retry="commentFeed.load(commentTarget)"
    />

    <EmptyState
      v-else-if="commentEntries.length === 0"
      compact
      icon="message-square"
      title="No comments yet"
      hint="Start the conversation — comments support markdown."
    />

    <div v-else class="flex flex-col" style="gap: 12px;">
      <template v-for="entry in commentEntries" :key="entry.type === 'comment' ? `comment:${entry.comment.id}` : `event:${entry.id}`">
        <CommentCard
          v-if="entry.type === 'comment'"
          :comment="entry.comment"
          :can-edit="canEdit(entry.comment)"
          :can-delete="canDelete(entry.comment)"
          :on-save="onSave"
          :on-delete="onDelete"
          :links="entry.links"
          :attachments="commentAttachmentItems[entry.comment.id] ?? []"
          :can-manage-attachments="canDelete(entry.comment)"
          :attachment-uploading="isAttachmentUploading(entry.comment.id)"
          :attachment-listing="isAttachmentListing(entry.comment.id)"
          :attachment-error="commentAttachmentError[entry.comment.id]"
          :on-reload-attachments="() => reloadCommentAttachments(entry.comment.id)"
          :is-attachment-downloading="(attachmentId) => isAttachmentDownloading(`${entry.comment.id}:${attachmentId}`)"
          :is-attachment-deleting="(attachmentId) => isAttachmentDeleting(`${entry.comment.id}:${attachmentId}`)"
          :on-upload-attachment="(file) => uploadCommentAttachment(entry.comment.id, file)"
          :on-download-attachment="(attachmentId) => downloadCommentAttachment(entry.comment.id, attachmentId)"
          :on-delete-attachment="(attachmentId) => deleteCommentAttachment(entry.comment.id, attachmentId)"
          :upload-image="canEdit(entry.comment) && canDelete(entry.comment) ? (file) => uploadImage(entry.comment.id, file) : undefined"
          @navigate-link="navigateCommentTarget"
        />
        <CommentCard v-else :event="entry" @navigate-link="navigateCommentTarget" />
      </template>
    </div>

    <div v-if="commentStatus !== 'error' && commentHasMore" style="margin-top: 12px;">
      <button
        type="button"
        data-test="comment-load-more"
        class="atl-comment-btn"
        @click="loadMore"
      >
        Load more comments
      </button>
    </div>

    <CommentComposer v-if="commentStatus !== 'error'" :target="commentTarget" :on-submit="onSubmit" />
  </section>
</template>

<style scoped>
.atl-ac {
  display: flex;
  flex-direction: column;
}
</style>
