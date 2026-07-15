<script setup lang="ts">
/**
 * Presentational card for a single comment, shared by the task and document
 * comment threads. It owns its inline-edit state, its actions (⋯) menu, and its
 * delete confirmation; the caller controls the affordances via `canEdit` /
 * `canDelete` and performs the mutations via `onSave` / `onDelete` (reacting to
 * failures, e.g. an error banner). Edit mode is left only on a successful save.
 */
import { computed, ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import MarkdownEditor from '@/components/editor/MarkdownEditor.vue';
import AgentBadge from '@/components/ui/AgentBadge.vue';
import Avatar from '@/components/ui/Avatar.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import ContextMenu, { type MenuItem } from '@/components/ui/ContextMenu.vue';
import Icon from '@/components/ui/Icon.vue';
import { useContextMenu } from '@/composables/useContextMenu';
import { actorName, isAgent } from '@/lib/actor';
import { formatDate } from '@/lib/format';

type CommentDto = components['schemas']['CommentDto'];

type CommentLinkTarget =
  | { status: 'available'; id: string; type: string; label?: string }
  | { status: 'unavailable'; label: 'Recurso no disponible' };

type CommentAttachment = {
  id: string;
  comment_id: string;
  content_type: string;
  created_at: string;
  file_name: string;
  size_bytes: number;
};

type CommentEvent = {
  id: string;
  comment_id?: string;
  kind: string;
  created_at: string;
  target?: CommentLinkTarget | null;
};

const props = defineProps<{
  comment?: CommentDto;
  canEdit?: boolean;
  canDelete?: boolean;
  onSave?: (id: string, body: string) => Promise<boolean>;
  onDelete?: (id: string) => Promise<void> | Promise<boolean>;
  links?: Array<{ target: CommentLinkTarget }>;
  event?: CommentEvent;
  attachments?: CommentAttachment[];
  canManageAttachments?: boolean;
  attachmentUploading?: boolean;
  attachmentError?: string | null;
  onUploadAttachment?: (file: File) => Promise<CommentAttachment | null>;
  onDownloadAttachment?: (attachmentId: string) => Promise<Blob | null>;
  onDeleteAttachment?: (attachmentId: string) => Promise<boolean>;
  uploadImage?: (file: File) => Promise<string | null>;
}>();

const emit = defineEmits<{
  'navigate-link': [target: Extract<CommentLinkTarget, { status: 'available' }>, commentId?: string];
}>();

const menu = useContextMenu();
const pendingDelete = ref(false);
const pendingAttachmentDelete = ref<CommentAttachment | null>(null);

const editing = ref(false);
const editDraft = ref('');
const saving = ref(false);

const canSaveEdit = computed(() => editDraft.value.trim().length > 0);
const hasActions = computed(() => props.canEdit === true || props.canDelete === true);
const isEdited = computed(
  () => props.comment !== undefined && props.comment.updated_at !== props.comment.created_at,
);

const authorName = computed(() =>
  props.comment === undefined ? '' : actorName(props.comment.author.display_name, props.comment.author.type),
);
const authorIsAgent = computed(() => props.comment !== undefined && isAgent(props.comment.author.type));

function startEdit(): void {
  if (props.comment === undefined) return;

  editing.value = true;
  editDraft.value = props.comment.body;
}

function cancelEdit(): void {
  editing.value = false;
  editDraft.value = '';
}

function requestDelete(): void {
  pendingDelete.value = true;
}

function linkLabel(target: CommentLinkTarget): string {
  if (target.status === 'unavailable') return 'Recurso no disponible';
  if (target.label !== undefined) return target.label;

  if (target.type === 'task') return 'Task link';
  if (target.type === 'document') return 'Note link';
  if (target.type === 'attachment') return 'Attachment link';
  return 'Linked resource';
}

function eventLabel(kind: string): string {
  if (kind === 'link_added') return 'Link added';
  if (kind === 'link_removed') return 'Link removed';
  if (kind === 'comment_deleted') return 'Comment deleted';
  return 'Comment activity';
}

function navigate(target: CommentLinkTarget): void {
  if (target.status === 'available') {
    emit('navigate-link', target, props.comment?.id ?? props.event?.comment_id);
  }
}

async function uploadAttachment(event: Event): Promise<void> {
  const input = event.target as HTMLInputElement;
  const file = input.files?.[0];
  input.value = '';

  if (file !== undefined && props.onUploadAttachment !== undefined) {
    await props.onUploadAttachment(file);
  }
}

async function downloadAttachment(attachmentId: string): Promise<void> {
  await props.onDownloadAttachment?.(attachmentId);
}

function requestAttachmentDelete(attachment: CommentAttachment): void {
  pendingAttachmentDelete.value = attachment;
}

async function confirmAttachmentDelete(): Promise<void> {
  const attachment = pendingAttachmentDelete.value;
  pendingAttachmentDelete.value = null;

  if (attachment !== null) await props.onDeleteAttachment?.(attachment.id);
}

const menuItems = computed<MenuItem[]>(() => {
  const items: MenuItem[] = [];

  if (props.canEdit) {
    items.push({ label: 'Edit', icon: 'pencil', action: startEdit });
  }
  if (props.canDelete) {
    items.push({ label: 'Delete', icon: 'trash', danger: true, action: requestDelete });
  }

  return items;
});

function onEditChange(markdown: string): void {
  editDraft.value = markdown;
}

async function saveEdit(): Promise<void> {
  if (!canSaveEdit.value || saving.value || props.comment === undefined || props.onSave === undefined) return;

  saving.value = true;
  const ok = await props.onSave(props.comment.id, editDraft.value);
  saving.value = false;

  if (ok) cancelEdit();
}

function cancelDelete(): void {
  pendingDelete.value = false;
}

async function confirmDelete(): Promise<void> {
  pendingDelete.value = false;
  if (props.comment !== undefined && props.onDelete !== undefined) await props.onDelete(props.comment.id);
}
</script>

<template>
  <article
    v-if="comment === undefined && event !== undefined"
    :data-comment-event="event.id"
    class="atl-comment"
  >
    <div style="color: var(--c-muted);">
      {{ eventLabel(event.kind) }}
      <button
        v-if="event.target?.status === 'available'"
        type="button"
        class="atl-comment-btn"
        @click="navigate(event.target)"
      >
        {{ linkLabel(event.target) }}
      </button>
      <span v-else-if="event.target?.status === 'unavailable'">{{ linkLabel(event.target) }}</span>
    </div>
    <span style="font-size: var(--fs-xs); color: var(--c-muted);">{{ formatDate(event.created_at) }}</span>
  </article>

  <article v-else-if="comment !== undefined" :data-comment-id="comment.id" class="atl-comment group">
    <div class="flex items-center" style="gap: 8px;">
      <Avatar :name="authorName" :agent="authorIsAgent" :size="22" />
      <span
        style="font-family: var(--font-mono); font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--c-foreground);"
      >
        {{ authorName }}
      </span>
      <AgentBadge v-if="authorIsAgent" />
      <span style="font-size: var(--fs-xs); color: var(--c-muted);">
        {{ formatDate(comment.created_at) }}
      </span>
      <span v-if="isEdited" style="font-size: var(--fs-xs); color: var(--c-muted);">
        (edited)
      </span>
      <span style="flex: 1;" />
      <button
        v-if="hasActions"
        type="button"
        aria-label="Comment actions"
        title="Comment actions"
        class="shrink-0 cursor-pointer opacity-0 group-hover:opacity-100 focus-visible:opacity-100 flex items-center justify-center"
        style="width: 22px; height: 22px; border: 1px solid var(--c-border); border-radius: var(--r-sm); background: var(--c-secondary); color: var(--c-muted);"
        @click="menu.openAt($event)"
      >
        <Icon name="ellipsis" :size="13" />
      </button>
    </div>

    <div style="margin-top: 4px; margin-left: 30px;">
      <template v-if="editing">
        <MarkdownEditor
          :body="editDraft"
          :editable="true"
          :embedded-controls="false"
          :width-toggle="false"
          :follow-caret="false"
          :upload-image="uploadImage"
          min-height="2.5rem"
          @change="onEditChange"
        />
        <div class="flex justify-end" style="gap: 8px; margin-top: 8px;">
          <button
            type="button"
            data-test="comment-edit-cancel"
            class="atl-comment-btn"
            @click="cancelEdit"
          >
            Cancel
          </button>
          <button
            type="button"
            data-test="comment-edit-save"
            class="atl-comment-submit"
            :disabled="!canSaveEdit || saving"
            @click="saveEdit"
          >
            {{ saving ? 'Saving…' : 'Save' }}
          </button>
        </div>
      </template>
      <MarkdownEditor
        v-else
        :body="comment.body"
        :editable="false"
        :reading="true"
        :embedded-controls="false"
        :width-toggle="false"
        min-height="1rem"
      />

      <div v-if="links !== undefined && links.length > 0" class="flex flex-wrap" style="gap: 6px; margin-top: 8px;">
        <template v-for="link in links" :key="link.target.status === 'available' ? link.target.id : link.target.label">
          <button
            v-if="link.target.status === 'available'"
            type="button"
            :data-comment-link="link.target.id"
            class="atl-comment-btn"
            @click="navigate(link.target)"
          >
            {{ linkLabel(link.target) }}
          </button>
          <span v-else data-comment-link-unavailable>{{ linkLabel(link.target) }}</span>
        </template>
      </div>

      <div v-if="event !== undefined" :data-comment-event="event.id" style="margin-top: 8px; color: var(--c-muted);">
        {{ eventLabel(event.kind) }}
        <button
          v-if="event.target?.status === 'available'"
          type="button"
          class="atl-comment-btn"
          @click="navigate(event.target)"
        >
          {{ linkLabel(event.target) }}
        </button>
        <span v-else-if="event.target?.status === 'unavailable'">{{ linkLabel(event.target) }}</span>
      </div>

      <div v-if="attachments !== undefined" style="margin-top: 8px;">
        <label
          v-if="canManageAttachments"
          class="atl-comment-btn"
          :for="`comment-attachment-picker-${comment.id}`"
        >
          Attach file
        </label>
        <input
          v-if="canManageAttachments"
          :id="`comment-attachment-picker-${comment.id}`"
          data-comment-attachment-picker
          class="sr-only"
          type="file"
          aria-label="Attach file"
          @change="uploadAttachment"
        />
        <span v-if="attachmentUploading" role="status" style="margin-left: 8px;">Uploading attachment…</span>
        <p v-if="attachmentError !== null && attachmentError !== undefined" role="alert">{{ attachmentError }}</p>
        <ul v-if="attachments.length > 0" style="margin-top: 6px;">
          <li v-for="attachment in attachments" :key="attachment.id" class="flex items-center" style="gap: 6px;">
            <span>{{ attachment.file_name }}</span>
            <button
              type="button"
              class="atl-comment-btn"
              :aria-label="`Download ${attachment.file_name}`"
              @click="downloadAttachment(attachment.id)"
            >
              Download
            </button>
            <button
              v-if="canManageAttachments"
              type="button"
              class="atl-comment-btn"
              :aria-label="`Delete ${attachment.file_name}`"
              @click="requestAttachmentDelete(attachment)"
            >
              Delete
            </button>
          </li>
        </ul>
      </div>
    </div>

    <ContextMenu
      :open="menu.open.value"
      :x="menu.x.value"
      :y="menu.y.value"
      :items="menuItems"
      :width="160"
      @close="menu.close()"
    />

    <ConfirmDialog
      :open="pendingDelete"
      title="Delete comment"
      message="This permanently removes the comment. This cannot be undone."
      tone="danger"
      confirm-label="Delete"
      @confirm="confirmDelete"
      @cancel="cancelDelete"
    />

    <ConfirmDialog
      :open="pendingAttachmentDelete !== null"
      title="Delete attachment"
      message="This permanently removes the attachment. This cannot be undone."
      tone="danger"
      confirm-label="Delete"
      @confirm="confirmAttachmentDelete"
      @cancel="pendingAttachmentDelete = null"
    />
  </article>
</template>
