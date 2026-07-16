<script setup lang="ts">
import { ref } from 'vue';
import Icon from '@/components/ui/Icon.vue';
import PromptDialog from '@/components/ui/PromptDialog.vue';
import { formatBytes } from '@/lib/format';
import { type TaskAttachmentDto, useTaskDetailStore } from '@/stores/taskDetail';
import { useUiStore } from '@/stores/ui';

const props = defineProps<{
  attachments: TaskAttachmentDto[];
  ws: string;
  readableId: string;
}>();

const emit = defineEmits<{
  remove: [attachmentId: string];
}>();

const detail = useTaskDetailStore();
const ui = useUiStore();
const renameTarget = ref<TaskAttachmentDto | null>(null);
const pendingRenameGeneration = ref<number | null>(null);
const renameError = ref('');
let renameGeneration = 0;

function startRename(attachment: TaskAttachmentDto): void {
  renameGeneration += 1;
  renameError.value = '';
  renameTarget.value = attachment;
}

function closeRename(): void {
  renameGeneration += 1;
  renameError.value = '';
  renameTarget.value = null;
}

async function submitRename(fileName: string): Promise<void> {
  const target = renameTarget.value;
  const generation = renameGeneration;
  const trimmedFileName = fileName.trim();
  if (target === null || pendingRenameGeneration.value === generation) return;

  if (trimmedFileName === '') {
    renameError.value = 'file_name must not be blank';
    return;
  }
  if (new TextEncoder().encode(trimmedFileName).length > 200) {
    renameError.value = 'file_name must be at most 200 bytes';
    return;
  }

  renameError.value = '';
  if (trimmedFileName === target.file_name) {
    closeRename();
    return;
  }

  pendingRenameGeneration.value = generation;
  let ok = false;
  try {
    ok = await detail.renameAttachment(props.ws, props.readableId, target.id, trimmedFileName);
  } finally {
    if (pendingRenameGeneration.value === generation) pendingRenameGeneration.value = null;
  }

  if (generation !== renameGeneration || renameTarget.value?.id !== target.id) return;
  if (ok) closeRename();
  else if (detail.error) ui.showBanner(detail.error, 'error');
}

/**
 * The download streams through the API (cookie-authenticated, same origin), and
 * the endpoint sets Content-Disposition so the browser saves rather than navigates.
 */
function contentUrl(attachmentId: string): string {
  return `/api/workspaces/${props.ws}/tasks/${props.readableId}/attachments/${attachmentId}/content`;
}

function isImage(att: TaskAttachmentDto): boolean {
  return att.content_type.startsWith('image/');
}
</script>

<template>
  <div class="flex flex-col" style="gap: 6px;">
    <div
      v-for="att in attachments"
      :key="att.id"
      class="flex flex-col"
      style="gap: 6px;"
      :data-attachment-id="att.id"
    >
      <div class="group flex items-center" style="gap: 8px;">
        <Icon name="paperclip" :size="14" style="color: var(--c-muted); flex: 0 0 auto;" />
        <a
          :href="contentUrl(att.id)"
          :download="att.file_name"
          class="flex-1 min-w-0 truncate atl-att-name"
          :title="`Download ${att.file_name}`"
        >
          {{ att.file_name }}
        </a>
        <span style="flex: 0 0 auto; font-size: var(--fs-xs); color: var(--c-muted);">
          {{ formatBytes(att.size_bytes) }}
        </span>
        <div
          class="inline-flex items-center opacity-0 group-hover:opacity-100 group-focus-within:opacity-100"
          style="gap: 4px;"
        >
          <button
            type="button"
            :aria-label="`Rename attachment ${att.file_name}`"
            class="inline-flex items-center justify-center cursor-pointer"
            style="width: 24px; height: 24px; border: none; background: transparent; color: var(--c-muted); padding: 0;"
            @click="startRename(att)"
          >
            <Icon name="pencil" :size="12" />
          </button>
          <button
            type="button"
            :aria-label="`Remove attachment ${att.file_name}`"
            class="inline-flex items-center justify-center cursor-pointer"
            style="width: 16px; height: 16px; border: none; background: transparent; color: var(--c-muted); padding: 0;"
            @click="emit('remove', att.id)"
          >
            <Icon name="x" :size="13" />
          </button>
        </div>
      </div>

      <a
        v-if="isImage(att)"
        :href="contentUrl(att.id)"
        target="_blank"
        rel="noopener"
        class="atl-att-thumb"
        :title="att.file_name"
      >
        <img :src="contentUrl(att.id)" :alt="att.file_name" loading="lazy" />
      </a>
    </div>

    <PromptDialog
      :open="renameTarget !== null"
      title="Rename attachment"
      :initial="renameTarget?.file_name ?? ''"
      placeholder="File name"
      confirm-label="Rename"
      :error="renameError"
      @confirm="submitRename"
      @cancel="closeRename"
    />
  </div>
</template>

<style scoped>
.atl-att-name {
  font-size: var(--fs-sm);
  color: var(--c-foreground);
}

.atl-att-name:hover {
  color: var(--c-primary);
  text-decoration: underline;
}

.atl-att-thumb {
  display: block;
  align-self: flex-start;
  margin-left: 22px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  overflow: hidden;
}

.atl-att-thumb img {
  display: block;
  max-width: 240px;
  max-height: 160px;
  object-fit: contain;
}
</style>
