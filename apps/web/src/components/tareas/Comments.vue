<script setup lang="ts">
import { computed, ref } from 'vue';
import MarkdownEditor from '@/components/editor/MarkdownEditor.vue';
import EmptyState from '@/components/states/EmptyState.vue';
import AgentBadge from '@/components/ui/AgentBadge.vue';
import Avatar from '@/components/ui/Avatar.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import ContextMenu, { type MenuItem } from '@/components/ui/ContextMenu.vue';
import Icon from '@/components/ui/Icon.vue';
import { useContextMenu } from '@/composables/useContextMenu';
import { formatDate } from '@/lib/format';
import { useAuthStore } from '@/stores/auth';
import { type CommentDto, useTaskDetailStore } from '@/stores/taskDetail';
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

const draft = ref('');
const submitting = ref(false);
const pendingDelete = ref<CommentDto | null>(null);

const composerEditor = ref<{ focus: () => void } | null>(null);

const menu = useContextMenu();
const menuComment = ref<CommentDto | null>(null);

const editingId = ref<string | null>(null);
const editDraft = ref('');
const savingEdit = ref(false);

const canSubmit = computed(() => draft.value.trim().length > 0);
const canSaveEdit = computed(() => editDraft.value.trim().length > 0);

// The server authorizes deletion (author OR workspace admin/owner); this only
// gates whether the affordance is shown. A break-glass global admin with no
// membership row here sees no button and would get a 403, which is acceptable.
const canModerate = computed(
  () => workspace.myWorkspaceRole === 'owner' || workspace.myWorkspaceRole === 'admin',
);

const currentActorId = computed(() => auth.user?.id ?? null);

function isAgent(actorType: string): boolean {
  return actorType === 'api_key';
}

function authorName(comment: CommentDto): string {
  return comment.author.display_name ?? (isAgent(comment.author.type) ? 'Agent' : 'User');
}

// Editing is author-only: the server forbids admins from editing others' comments.
function canEdit(comment: CommentDto): boolean {
  return currentActorId.value !== null && comment.author.id === currentActorId.value;
}

function canDelete(comment: CommentDto): boolean {
  if (canModerate.value) return true;
  return currentActorId.value !== null && comment.author.id === currentActorId.value;
}

function hasActions(comment: CommentDto): boolean {
  return canEdit(comment) || canDelete(comment);
}

function isEdited(comment: CommentDto): boolean {
  return comment.updated_at !== comment.created_at;
}

const menuItems = computed<MenuItem[]>(() => {
  const comment = menuComment.value;
  if (comment === null) return [];

  const items: MenuItem[] = [];

  if (canEdit(comment)) {
    items.push({ label: 'Edit', icon: 'pencil', action: () => startEdit(comment) });
  }
  if (canDelete(comment)) {
    items.push({ label: 'Delete', icon: 'trash', danger: true, action: () => requestDelete(comment) });
  }

  return items;
});

function openMenu(comment: CommentDto, event: MouseEvent): void {
  menuComment.value = comment;
  menu.openAt(event);
}

function focusComposer(): void {
  composerEditor.value?.focus?.();
}

function onDraftChange(markdown: string): void {
  draft.value = markdown;
}

function onEditChange(markdown: string): void {
  editDraft.value = markdown;
}

function startEdit(comment: CommentDto): void {
  editingId.value = comment.id;
  editDraft.value = comment.body;
}

function cancelEdit(): void {
  editingId.value = null;
  editDraft.value = '';
}

async function saveEdit(comment: CommentDto): Promise<void> {
  if (!canSaveEdit.value || savingEdit.value) return;

  savingEdit.value = true;
  const ok = await detail.editComment(props.ws, props.readableId, comment.id, editDraft.value.trim());
  savingEdit.value = false;

  if (ok) cancelEdit();
  else if (detail.error) ui.showBanner(detail.error, 'error');
}

async function submit(): Promise<void> {
  if (!canSubmit.value || submitting.value) return;

  submitting.value = true;
  const ok = await detail.addComment(props.ws, props.readableId, draft.value.trim());
  submitting.value = false;

  if (ok) draft.value = '';
  else if (detail.error) ui.showBanner(detail.error, 'error');
}

function requestDelete(comment: CommentDto): void {
  pendingDelete.value = comment;
}

function cancelDelete(): void {
  pendingDelete.value = null;
}

async function confirmDelete(): Promise<void> {
  const comment = pendingDelete.value;
  pendingDelete.value = null;
  if (comment === null) return;

  const ok = await detail.removeComment(props.ws, props.readableId, comment.id);
  if (!ok && detail.error) ui.showBanner(detail.error, 'error');
}

async function loadMore(): Promise<void> {
  await detail.loadMoreComments(props.ws, props.readableId);
  if (detail.error) ui.showBanner(detail.error, 'error');
}
</script>

<template>
  <section>
    <EmptyState
      v-if="detail.comments.length === 0"
      compact
      icon="message-square"
      title="No comments yet"
      hint="Start the conversation — comments support markdown."
    />

    <div v-else class="flex flex-col" style="gap: 14px;">
      <article
        v-for="comment in detail.comments"
        :key="comment.id"
        :data-comment-id="comment.id"
        class="group"
      >
        <div class="flex items-center" style="gap: 8px;">
          <Avatar :name="authorName(comment)" :agent="isAgent(comment.author.type)" :size="22" />
          <span
            style="font-family: var(--font-mono); font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--c-foreground);"
          >
            {{ authorName(comment) }}
          </span>
          <AgentBadge v-if="isAgent(comment.author.type)" />
          <span style="font-size: var(--fs-xs); color: var(--c-muted);">
            {{ formatDate(comment.created_at) }}
          </span>
          <span v-if="isEdited(comment)" style="font-size: var(--fs-xs); color: var(--c-muted);">
            (edited)
          </span>
          <span style="flex: 1;" />
          <button
            v-if="hasActions(comment)"
            type="button"
            aria-label="Comment actions"
            title="Comment actions"
            class="shrink-0 cursor-pointer opacity-0 group-hover:opacity-100 flex items-center justify-center"
            style="width: 22px; height: 22px; border: 1px solid var(--c-border); border-radius: var(--r-sm); background: var(--c-secondary); color: var(--c-muted);"
            @click="openMenu(comment, $event)"
          >
            <Icon name="ellipsis" :size="13" />
          </button>
        </div>

        <div style="margin-top: 4px; margin-left: 30px;">
          <template v-if="editingId === comment.id">
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
                class="atl-comment-loadmore"
                @click="cancelEdit"
              >
                Cancel
              </button>
              <button
                type="button"
                data-test="comment-edit-save"
                class="atl-comment-submit"
                :disabled="!canSaveEdit || savingEdit"
                @click="saveEdit(comment)"
              >
                {{ savingEdit ? 'Saving…' : 'Save' }}
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
      </article>
    </div>

    <div v-if="detail.commentsHasMore" style="margin-top: 12px;">
      <button
        type="button"
        data-test="comment-load-more"
        class="atl-comment-loadmore"
        @click="loadMore"
      >
        Load more comments
      </button>
    </div>

    <div data-comment-composer class="atl-comment-composer" @click="focusComposer">
      <MarkdownEditor
        ref="composerEditor"
        :body="draft"
        :editable="true"
        :embedded-controls="false"
        :width-toggle="false"
        min-height="1.75rem"
        placeholder="Write a comment…"
        @change="onDraftChange"
      />
      <div class="flex justify-end" style="margin-top: 8px;">
        <button
          type="button"
          data-test="comment-submit"
          class="atl-comment-submit"
          :disabled="!canSubmit || submitting"
          @click.stop="submit"
        >
          <Icon name="send" :size="13" />
          {{ submitting ? 'Posting…' : 'Comment' }}
        </button>
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
      :open="pendingDelete !== null"
      title="Delete comment"
      message="This permanently removes the comment. This cannot be undone."
      tone="danger"
      confirm-label="Delete"
      @confirm="confirmDelete"
      @cancel="cancelDelete"
    />
  </section>
</template>

<style scoped>
.atl-comment-composer {
  margin-top: 16px;
  padding: 10px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  background: var(--c-panel);
  cursor: text;
}

.atl-comment-loadmore {
  height: 28px;
  padding: 0 12px;
  background: transparent;
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  color: var(--c-foreground);
  font-family: var(--font-ui);
  font-size: var(--fs-sm);
  cursor: pointer;
}

.atl-comment-loadmore:hover {
  background: rgba(179, 177, 173, 0.06);
}

.atl-comment-submit {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 28px;
  padding: 0 12px;
  background: var(--c-primary);
  border: 1px solid var(--c-primary);
  border-radius: var(--r-md);
  color: var(--c-background);
  font-family: var(--font-ui);
  font-size: var(--fs-sm);
  font-weight: var(--fw-semibold);
  cursor: pointer;
}

.atl-comment-submit:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>
