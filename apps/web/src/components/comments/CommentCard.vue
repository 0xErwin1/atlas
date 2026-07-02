<script setup lang="ts">
/**
 * Presentational card for a single comment, shared by the task and document
 * comment threads. It owns its inline-edit state, its actions (⋯) menu, and its
 * delete confirmation; the caller controls the affordances via `canEdit` /
 * `canDelete` and performs the mutations via `onSave` / `onDelete` (reacting to
 * failures, e.g. an error banner). Edit mode is left only on a successful save.
 */
import { computed, ref } from 'vue';
import MarkdownEditor from '@/components/editor/MarkdownEditor.vue';
import AgentBadge from '@/components/ui/AgentBadge.vue';
import Avatar from '@/components/ui/Avatar.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import ContextMenu, { type MenuItem } from '@/components/ui/ContextMenu.vue';
import Icon from '@/components/ui/Icon.vue';
import { useContextMenu } from '@/composables/useContextMenu';
import { actorName, isAgent } from '@/lib/actor';
import { formatDate } from '@/lib/format';
import type { CommentDto } from '@/stores/documents';

const props = defineProps<{
  comment: CommentDto;
  canEdit: boolean;
  canDelete: boolean;
  onSave: (id: string, body: string) => Promise<boolean>;
  onDelete: (id: string) => Promise<void> | Promise<boolean>;
}>();

const menu = useContextMenu();
const pendingDelete = ref(false);

const editing = ref(false);
const editDraft = ref('');
const saving = ref(false);

const canSaveEdit = computed(() => editDraft.value.trim().length > 0);
const hasActions = computed(() => props.canEdit || props.canDelete);
const isEdited = computed(() => props.comment.updated_at !== props.comment.created_at);

const authorName = computed(() => actorName(props.comment.author.display_name, props.comment.author.type));
const authorIsAgent = computed(() => isAgent(props.comment.author.type));

function startEdit(): void {
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
  if (!canSaveEdit.value || saving.value) return;

  saving.value = true;
  const ok = await props.onSave(props.comment.id, editDraft.value.trim());
  saving.value = false;

  if (ok) cancelEdit();
}

function cancelDelete(): void {
  pendingDelete.value = false;
}

async function confirmDelete(): Promise<void> {
  pendingDelete.value = false;
  await props.onDelete(props.comment.id);
}
</script>

<template>
  <article :data-comment-id="comment.id" class="atl-comment group">
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
        class="shrink-0 cursor-pointer opacity-0 group-hover:opacity-100 flex items-center justify-center"
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
  </article>
</template>
