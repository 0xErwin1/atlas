<script setup lang="ts">
import Icon from '@/components/ui/Icon.vue';
import { formatBytes } from '@/lib/format';
import type { TaskAttachmentDto } from '@/stores/taskDetail';

const props = defineProps<{
  attachments: TaskAttachmentDto[];
  ws: string;
  readableId: string;
}>();

const emit = defineEmits<{
  remove: [attachmentId: string];
}>();

/**
 * The download streams through the API (cookie-authenticated, same origin), and
 * the endpoint sets Content-Disposition so the browser saves rather than navigates.
 */
function contentUrl(attachmentId: string): string {
  return `/v1/workspaces/${props.ws}/tasks/${props.readableId}/attachments/${attachmentId}/content`;
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
        <button
          type="button"
          :aria-label="`Remove attachment ${att.file_name}`"
          class="inline-flex items-center justify-center cursor-pointer opacity-0 group-hover:opacity-100"
          style="width: 16px; height: 16px; border: none; background: transparent; color: var(--c-muted); padding: 0;"
          @click="emit('remove', att.id)"
        >
          <Icon name="x" :size="13" />
        </button>
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
