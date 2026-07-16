<script setup lang="ts">
/**
 * Presentational comment composer: a markdown box with a submit button, shared by
 * the task and document comment threads. It owns its own draft and submitting
 * flag; the caller supplies `onSubmit` and reacts to failures (e.g. an error
 * banner). On a successful submit (resolved `true`) the draft is cleared.
 */
import { computed, ref } from 'vue';
import MarkdownEditor from '@/components/editor/MarkdownEditor.vue';
import Icon from '@/components/ui/Icon.vue';
import { useCommentDraftAttachments } from '@/composables/useCommentDraftAttachments';
import type { CommentParentTarget } from '@/composables/useCommentFeed';

const props = withDefaults(
  defineProps<{
    placeholder?: string;
    target: CommentParentTarget;
    onSubmit: (body: string, draftId?: string) => Promise<boolean>;
  }>(),
  {
    placeholder: 'Write a comment…',
  },
);

const draft = ref('');
const submitting = ref(false);
const submissionFailed = ref(false);
const fileInput = ref<HTMLInputElement | null>(null);
const draftAttachments = useCommentDraftAttachments(computed(() => props.target));
const attachments = draftAttachments.attachments;
const draftId = draftAttachments.draftId;
const draftError = draftAttachments.error;

const editor = ref<{ focus: () => void } | null>(null);

const uploading = computed(() =>
  draftAttachments.attachments.value.some(
    (attachment) =>
      attachment.status === 'queued' || attachment.status === 'uploading' || attachment.status === 'deleting',
  ),
);
const canSubmit = computed(() => draft.value.trim().length > 0 && !uploading.value);

function onDraftChange(markdown: string): void {
  draft.value = markdown;
}

function focus(): void {
  editor.value?.focus?.();
}

async function submit(): Promise<void> {
  if (!canSubmit.value || submitting.value) return;

  submitting.value = true;
  const id = draftAttachments.draftId.value ?? undefined;
  let ok = false;

  try {
    ok = id === undefined ? await props.onSubmit(draft.value) : await props.onSubmit(draft.value, id);
  } catch {
    ok = false;
  } finally {
    submitting.value = false;
  }

  if (ok) {
    draft.value = '';
    submissionFailed.value = false;
    draftAttachments.attachments.value = [];
    draftAttachments.draftId.value = null;
  } else {
    submissionFailed.value = true;
  }
}

function selectFiles(): void {
  fileInput.value?.click();
}

function onFilesSelected(event: Event): void {
  const input = event.target as HTMLInputElement;
  for (const file of Array.from(input.files ?? [])) void draftAttachments.enqueue(file);
  input.value = '';
}

async function discard(): Promise<void> {
  await draftAttachments.discard();
  if (draftAttachments.error.value === null) {
    draft.value = '';
    submissionFailed.value = false;
  }
}

defineExpose({ focus });
</script>

<template>
  <div data-comment-composer class="atl-comment-composer" @click="focus">
    <MarkdownEditor
      ref="editor"
      :body="draft"
      :editable="true"
      :embedded-controls="false"
      :width-toggle="false"
      :follow-caret="false"
      min-height="1.75rem"
      :placeholder="placeholder"
      :upload-image="draftAttachments.uploadImage"
      @change="onDraftChange"
    />
    <input
      ref="fileInput"
      class="sr-only"
      type="file"
      multiple
      aria-label="Add comment attachments"
      @change="onFilesSelected"
    />
    <div v-if="attachments.length > 0" aria-label="Comment draft attachments">
      <div v-for="attachment in attachments" :key="attachment.clientId" class="flex" style="gap: 8px;">
        <span>{{ attachment.file.name }}</span>
        <span v-if="attachment.status === 'uploading'" role="status">Uploading {{ attachment.progress ?? 0 }}%</span>
        <span v-else-if="attachment.status === 'queued'" role="status">Queued</span>
        <span v-else-if="attachment.status === 'uploaded'">Uploaded</span>
        <span v-else-if="attachment.status === 'deleting'" role="status">Deleting</span>
        <span v-else role="alert">{{ attachment.error }}</span>
        <button
          v-if="attachment.status === 'error'"
          type="button"
          :aria-label="attachment.attachment === null ? `Retry ${attachment.file.name}` : `Retry removal of ${attachment.file.name}`"
          @click.stop="attachment.attachment === null ? draftAttachments.retry(attachment.clientId) : draftAttachments.remove(attachment.clientId)"
        >
          Retry
        </button>
        <button type="button" :aria-label="`Remove ${attachment.file.name}`" @click.stop="draftAttachments.remove(attachment.clientId)">
          Remove
        </button>
      </div>
    </div>
    <p v-if="draftError" role="alert">{{ draftError }}</p>
    <div class="flex justify-end" style="margin-top: 8px;">
      <p v-if="submissionFailed" role="alert" style="margin-right: auto;">
        Could not post comment. Your text is still here; try again.
      </p>
      <button
        type="button"
        aria-label="Add comment attachments"
        @click.stop="selectFiles"
      >
        Attach files
      </button>
      <button
        v-if="draftId !== null || attachments.length > 0"
        type="button"
        aria-label="Discard comment draft"
        @click.stop="discard"
      >
        Discard
      </button>
      <button
        type="button"
        data-test="comment-submit"
        :aria-label="submissionFailed ? 'Retry comment' : 'Post comment'"
        class="atl-comment-submit"
        :disabled="!canSubmit || submitting"
        @click.stop="submit"
      >
        <Icon name="send" :size="13" />
        {{ submitting ? 'Posting…' : submissionFailed ? 'Retry comment' : 'Comment' }}
      </button>
    </div>
  </div>
</template>
