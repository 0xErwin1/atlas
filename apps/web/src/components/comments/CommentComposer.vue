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

const props = withDefaults(
  defineProps<{
    placeholder?: string;
    onSubmit: (body: string) => Promise<boolean>;
    uploadImage?: (file: File) => Promise<string | null>;
  }>(),
  {
    placeholder: 'Write a comment…',
  },
);

const draft = ref('');
const submitting = ref(false);

const editor = ref<{ focus: () => void } | null>(null);

const canSubmit = computed(() => draft.value.trim().length > 0);

function onDraftChange(markdown: string): void {
  draft.value = markdown;
}

function focus(): void {
  editor.value?.focus?.();
}

async function submit(): Promise<void> {
  if (!canSubmit.value || submitting.value) return;

  submitting.value = true;
  const ok = await props.onSubmit(draft.value);
  submitting.value = false;

  if (ok) draft.value = '';
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
      :upload-image="uploadImage"
      @change="onDraftChange"
    />
    <div class="flex justify-end" style="margin-top: 8px;">
      <button
        type="button"
        data-test="comment-submit"
        aria-label="Post comment"
        class="atl-comment-submit"
        :disabled="!canSubmit || submitting"
        @click.stop="submit"
      >
        <Icon name="send" :size="13" />
        {{ submitting ? 'Posting…' : 'Comment' }}
      </button>
    </div>
  </div>
</template>
