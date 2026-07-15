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
const commentAttachments = commentFeed.attachments;
const commentAttachmentError = commentFeed.attachmentError;
const commentTarget = computed<CommentParentTarget>(() => ({
  kind: 'document',
  ws: props.ws,
  slug: props.slug,
}));
const attachmentCommentsLoaded = new Set<string>();

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
  return `/api/workspaces/${props.ws}/documents/${props.slug}/comments/${commentId}/attachments/${attachmentId}`;
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
          :attachments="commentAttachments[entry.comment.id] ?? []"
          :can-manage-attachments="canDelete(entry.comment)"
          :attachment-uploading="commentFeed.isAttachmentUploadLoading(entry.comment.id)"
          :attachment-error="commentAttachmentError[entry.comment.id]"
          :on-upload-attachment="(file) => uploadCommentAttachment(entry.comment.id, file)"
          :on-download-attachment="(attachmentId) => downloadCommentAttachment(entry.comment.id, attachmentId)"
          :on-delete-attachment="(attachmentId) => deleteCommentAttachment(entry.comment.id, attachmentId)"
          :upload-image="async (file) => {
            const attachment = await uploadCommentAttachment(entry.comment.id, file);
            return attachment === null ? null : attachmentUrl(entry.comment.id, attachment.id);
          }"
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

    <CommentComposer v-if="commentStatus !== 'error'" :on-submit="onSubmit" />
  </section>
</template>

<style scoped>
.atl-ac {
  display: flex;
  flex-direction: column;
}
</style>
