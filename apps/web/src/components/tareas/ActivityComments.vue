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
import { activityVerb } from '@/lib/activityVerb';
import { formatDate } from '@/lib/format';
import { useAuthStore } from '@/stores/auth';
import { type ActivityEntryDto, type CommentDto, useTaskDetailStore } from '@/stores/taskDetail';
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

// One chronological feed: system activity entries and user comments interleaved
// by time (oldest first), with the composer pinned below. ISO timestamps sort
// lexicographically, so a string compare is chronological.
type FeedItem =
  | { kind: 'comment'; key: string; at: string; comment: CommentDto }
  | { kind: 'activity'; key: string; at: string; entry: ActivityEntryDto };

const feed = computed<FeedItem[]>(() => {
  const items: FeedItem[] = [
    ...detail.comments.map(
      (comment): FeedItem => ({ kind: 'comment', key: `c:${comment.id}`, at: comment.created_at, comment }),
    ),
    ...detail.activity.map(
      (entry): FeedItem => ({ kind: 'activity', key: `a:${entry.id}`, at: entry.created_at, entry }),
    ),
  ];

  return items.sort((a, b) => (a.at < b.at ? -1 : a.at > b.at ? 1 : 0));
});

const isEmpty = computed(() => detail.comments.length === 0 && detail.activity.length === 0);

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

function actorName(displayName: string | null | undefined, actorType: string): string {
  return displayName ?? (isAgent(actorType) ? 'Agent' : 'User');
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

function closeMenu(): void {
  menu.close();
  menuComment.value = null;
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
  <section class="atl-ac">
    <EmptyState
      v-if="isEmpty"
      compact
      icon="message-square"
      title="No activity yet"
      hint="Activity and comments show up here — comments support markdown."
    />

    <div v-else class="atl-ac-feed flex flex-col" style="gap: 12px;">
      <template v-for="item in feed" :key="item.key">
        <div
          v-if="item.kind === 'activity'"
          class="flex items-start"
          style="gap: 8px;"
          :data-activity-id="item.entry.id"
        >
          <Avatar
            :name="actorName(item.entry.actor.display_name, item.entry.actor.type)"
            :agent="isAgent(item.entry.actor.type)"
            :size="20"
          />
          <div class="flex flex-col" style="gap: 2px; min-width: 0;">
            <div class="flex items-center" style="gap: 6px; flex-wrap: wrap;">
              <span
                style="font-family: var(--font-mono); font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--c-foreground);"
              >
                {{ actorName(item.entry.actor.display_name, item.entry.actor.type) }}
              </span>
              <AgentBadge v-if="isAgent(item.entry.actor.type)" />
              <span style="font-size: var(--fs-sm); color: var(--c-muted);">{{ activityVerb(item.entry.kind) }}</span>
            </div>
            <span style="font-size: var(--fs-xs); color: var(--c-muted);">
              {{ formatDate(item.entry.created_at) }}
            </span>
          </div>
        </div>

        <article
          v-else
          :data-comment-id="item.comment.id"
          class="atl-ac-comment group"
        >
          <div class="flex items-center" style="gap: 8px;">
            <Avatar
              :name="actorName(item.comment.author.display_name, item.comment.author.type)"
              :agent="isAgent(item.comment.author.type)"
              :size="22"
            />
            <span
              style="font-family: var(--font-mono); font-size: var(--fs-sm); font-weight: var(--fw-semibold); color: var(--c-foreground);"
            >
              {{ actorName(item.comment.author.display_name, item.comment.author.type) }}
            </span>
            <AgentBadge v-if="isAgent(item.comment.author.type)" />
            <span style="font-size: var(--fs-xs); color: var(--c-muted);">
              {{ formatDate(item.comment.created_at) }}
            </span>
            <span v-if="isEdited(item.comment)" style="font-size: var(--fs-xs); color: var(--c-muted);">
              (edited)
            </span>
            <span style="flex: 1;" />
            <button
              v-if="hasActions(item.comment)"
              type="button"
              aria-label="Comment actions"
              title="Comment actions"
              class="shrink-0 cursor-pointer opacity-0 group-hover:opacity-100 flex items-center justify-center"
              style="width: 22px; height: 22px; border: 1px solid var(--c-border); border-radius: var(--r-sm); background: var(--c-secondary); color: var(--c-muted);"
              @click="openMenu(item.comment, $event)"
            >
              <Icon name="ellipsis" :size="13" />
            </button>
          </div>

          <div style="margin-top: 4px; margin-left: 30px;">
            <template v-if="editingId === item.comment.id">
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
                  class="atl-ac-btn"
                  @click="cancelEdit"
                >
                  Cancel
                </button>
                <button
                  type="button"
                  data-test="comment-edit-save"
                  class="atl-ac-submit"
                  :disabled="!canSaveEdit || savingEdit"
                  @click="saveEdit(item.comment)"
                >
                  {{ savingEdit ? 'Saving…' : 'Save' }}
                </button>
              </div>
            </template>
            <MarkdownEditor
              v-else
              :body="item.comment.body"
              :editable="false"
              :reading="true"
              :embedded-controls="false"
              :width-toggle="false"
              min-height="1rem"
            />
          </div>
        </article>
      </template>
    </div>

    <div v-if="detail.commentsHasMore" style="margin-top: 12px;">
      <button
        type="button"
        data-test="comment-load-more"
        class="atl-ac-btn"
        @click="loadMore"
      >
        Load more comments
      </button>
    </div>

    <div data-comment-composer class="atl-ac-composer" @click="focusComposer">
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
          class="atl-ac-submit"
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
      @close="closeMenu"
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
.atl-ac {
  display: flex;
  flex-direction: column;
}

.atl-ac-composer {
  margin-top: 16px;
  padding: 10px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  background: var(--c-panel);
  cursor: text;
}

.atl-ac-btn {
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

.atl-ac-btn:hover {
  background: rgba(179, 177, 173, 0.06);
}

.atl-ac-submit {
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

.atl-ac-submit:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}
</style>
