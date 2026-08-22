<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import { useRouter } from 'vue-router';
import { wrappedClient } from '@/api/wrapper';
import ExpandableRow from '@/components/settings/ExpandableRow.vue';
import PanelHeader from '@/components/settings/PanelHeader.vue';
import RowAction from '@/components/settings/RowAction.vue';
import SettingsTable from '@/components/settings/SettingsTable.vue';
import EmptyState from '@/components/states/EmptyState.vue';
import ErrorState from '@/components/states/ErrorState.vue';
import LoadingState from '@/components/states/LoadingState.vue';
import Avatar from '@/components/ui/Avatar.vue';
import Btn from '@/components/ui/Btn.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import Dropdown, { type DropdownOption } from '@/components/ui/Dropdown.vue';
import Icon from '@/components/ui/Icon.vue';
import PromptDialog from '@/components/ui/PromptDialog.vue';
import { saveDownload } from '@/lib/download';
import { formatBytes, formatDate, initials } from '@/lib/format';
import {
  type AttachmentFilter,
  type OwnerFilter,
  type TypeFilter,
  useAttachmentsStore,
  type WorkspaceAttachment,
} from '@/stores/attachments';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

const router = useRouter();
const attachments = useAttachmentsStore();
const workspace = useWorkspaceStore();
const ui = useUiStore();

const ws = computed(() => workspace.activeWorkspaceSlug ?? '');

const query = ref('');
const owner = ref<OwnerFilter>('all');
const type = ref<TypeFilter>('all');

const ownerOptions: DropdownOption[] = [
  { value: 'all', label: 'Everywhere', icon: 'files' },
  { value: 'document', label: 'Notes', icon: 'file-text' },
  { value: 'task', label: 'Tasks', icon: 'kanban' },
];

const typeOptions: DropdownOption[] = [
  { value: 'all', label: 'Any type' },
  { value: 'image', label: 'Images' },
  { value: 'other', label: 'Documents' },
];

function currentFilter(): AttachmentFilter {
  return { query: query.value, owner: owner.value, type: type.value };
}

function refresh(): void {
  void attachments.load(ws.value, currentFilter());
}

// The name filter runs server-side, so typing is debounced into one request per
// pause rather than one per keystroke.
let queryTimer: ReturnType<typeof setTimeout> | null = null;

watch(query, () => {
  if (queryTimer !== null) clearTimeout(queryTimer);
  queryTimer = setTimeout(refresh, 250);
});

watch([ws, owner, type], refresh, { immediate: true });

function isImage(attachment: WorkspaceAttachment): boolean {
  return attachment.content_type.startsWith('image/');
}

function ownerLabel(attachment: WorkspaceAttachment): string {
  const { owner: source } = attachment;
  const base = source.kind === 'task' ? (source.task_readable_id ?? source.title) : source.title;
  return source.comment_id === undefined ? base : `${base} · comment`;
}

function openOwner(attachment: WorkspaceAttachment): void {
  const { owner: source } = attachment;

  if (source.kind === 'task' && source.task_readable_id !== undefined) {
    void router.push({ name: 'task-detail', params: { readableId: source.task_readable_id } });
    return;
  }
  if (source.kind === 'document' && source.document_slug !== undefined) {
    void router.push({ name: 'notes', params: { slug: source.document_slug } });
  }
}

const downloading = ref<Set<string>>(new Set());

/**
 * Fetches the bytes through the API client rather than following a plain link:
 * on desktop the webview resolves a `/api/…` href against its own asset origin,
 * which serves no API and carries no session.
 */
async function download(attachment: WorkspaceAttachment): Promise<void> {
  if (downloading.value.has(attachment.id)) return;

  downloading.value = new Set(downloading.value).add(attachment.id);
  try {
    const { data } = await wrappedClient.GET('/api/workspaces/{ws}/attachments/{attachment_id}', {
      params: { path: { ws: ws.value, attachment_id: attachment.id } },
      parseAs: 'blob',
    });

    if (data === undefined || !(await saveDownload(data, attachment.file_name))) {
      ui.showBanner(`Could not download ${attachment.file_name}`, 'error');
    }
  } finally {
    const next = new Set(downloading.value);
    next.delete(attachment.id);
    downloading.value = next;
  }
}

const expandedId = ref<string | null>(null);
const renameTarget = ref<WorkspaceAttachment | null>(null);
const renameError = ref('');
const renamePending = ref(false);
const deleteTarget = ref<WorkspaceAttachment | null>(null);

function startRename(attachment: WorkspaceAttachment): void {
  renameError.value = '';
  renameTarget.value = attachment;
}

function cancelRename(): void {
  renameError.value = '';
  renameTarget.value = null;
}

async function submitRename(fileName: string): Promise<void> {
  const target = renameTarget.value;
  if (target === null || renamePending.value) return;

  const trimmed = fileName.trim();
  if (trimmed === '') {
    renameError.value = 'file_name must not be blank';
    return;
  }
  if (trimmed === target.file_name) {
    cancelRename();
    return;
  }

  renamePending.value = true;
  let renamed = false;
  try {
    renamed = await attachments.rename(ws.value, target.id, trimmed);
  } finally {
    renamePending.value = false;
  }

  if (renamed) {
    cancelRename();
    ui.showBanner(`Renamed to ${trimmed}. References were updated.`, 'success');
    return;
  }

  renameError.value = attachments.error ?? 'Failed to rename file';
}

async function confirmDelete(): Promise<void> {
  const target = deleteTarget.value;
  if (target === null) return;

  const removed = await attachments.remove(ws.value, target.id);
  deleteTarget.value = null;

  if (!removed && attachments.error !== null) ui.showBanner(attachments.error, 'error');
}

const showEmptyState = computed(
  () => !attachments.loading && attachments.error === null && attachments.items.length === 0,
);
</script>

<template>
  <div class="atl-files">
    <PanelHeader title="Files" subtitle="Every file attached anywhere in this workspace">
      <template #actions>
        <Btn :disabled="attachments.loading" @click="refresh">
          <Icon name="refresh-cw" :size="14" /> Refresh
        </Btn>
      </template>
    </PanelHeader>

    <div class="atl-files-filters">
      <input
        v-model="query"
        type="search"
        class="atl-files-search"
        placeholder="Search file names…"
        aria-label="Search file names"
      />
      <Dropdown v-model="owner" :options="ownerOptions" aria-label="Filter by location" />
      <Dropdown v-model="type" :options="typeOptions" aria-label="Filter by file type" />
    </div>

    <ErrorState
      v-if="attachments.error !== null"
      title="Couldn’t load files"
      :hint="attachments.error"
      @retry="refresh"
    />
    <LoadingState v-else-if="attachments.loading && attachments.items.length === 0" label="Loading files…" />
    <EmptyState
      v-else-if="showEmptyState"
      compact
      icon="paperclip"
      title="No files match these filters."
    />

    <SettingsTable v-else-if="attachments.items.length > 0">
      <template #head>
        <div style="flex: 1;">Name</div>
        <div style="flex: 0 0 180px;">Attached to</div>
        <div style="flex: 0 0 160px;">Uploaded by</div>
        <div style="flex: 0 0 120px;">Uploaded</div>
        <div style="flex: 0 0 90px;">Size</div>
        <div style="flex: 0 0 190px;"></div>
      </template>

      <ExpandableRow
        v-for="attachment in attachments.items"
        :key="attachment.id"
        :expanded="expandedId === attachment.id"
        :data-attachment-id="attachment.id"
        style="--erow-actions-basis: 190px; min-height: 46px;"
        @toggle="expandedId = expandedId === attachment.id ? null : attachment.id"
      >
        <template #summary>
          <div class="atl-files-name">
            <Icon :name="isImage(attachment) ? 'image' : 'paperclip'" :size="14" />
            <span class="truncate">{{ attachment.file_name }}</span>
          </div>

          <div style="flex: 0 0 180px; min-width: 0;">
            <button
              type="button"
              class="atl-files-owner"
              :title="`Open ${ownerLabel(attachment)}`"
              @click.stop="openOwner(attachment)"
            >
              <Icon :name="attachment.owner.kind === 'task' ? 'kanban' : 'file-text'" :size="13" />
              <span class="truncate">{{ ownerLabel(attachment) }}</span>
            </button>
          </div>

          <div class="atl-files-actor">
            <Avatar
              :name="initials(attachment.actor?.display_name)"
              :size="20"
              :agent="attachment.actor?.type === 'api_key'"
            />
            <span class="truncate">{{ attachment.actor?.display_name ?? 'Unknown' }}</span>
          </div>

          <span class="atl-files-meta" style="flex: 0 0 120px;">{{ formatDate(attachment.created_at) }}</span>
          <span class="atl-files-meta" style="flex: 0 0 90px;">{{ formatBytes(attachment.size_bytes) }}</span>
        </template>

        <template #actions>
          <RowAction
            title="Download"
            :disabled="downloading.has(attachment.id)"
            @click="download(attachment)"
          >
            <Icon name="download" :size="13" />
          </RowAction>
          <RowAction title="Rename" @click="startRename(attachment)">
            <Icon name="pencil" :size="13" />
          </RowAction>
          <RowAction tone="danger" title="Delete" @click="deleteTarget = attachment">
            <Icon name="trash-2" :size="13" />
          </RowAction>
        </template>

        <template #panel>
          <dl class="atl-files-detail">
            <dt>Type</dt>
            <dd>{{ attachment.content_type }}</dd>
            <dt>Checksum</dt>
            <dd><code>{{ attachment.sha256 }}</code></dd>
            <dt>Last change</dt>
            <dd>{{ formatDate(attachment.updated_at) }}</dd>
            <dt>Attached to</dt>
            <dd>{{ attachment.owner.title }}</dd>
          </dl>
        </template>
      </ExpandableRow>
    </SettingsTable>

    <Btn v-if="attachments.hasMore" style="margin-top: 10px;" @click="attachments.loadMore(ws)">
      Load more
    </Btn>

    <PromptDialog
      :open="renameTarget !== null"
      title="Rename file"
      :initial="renameTarget?.file_name ?? ''"
      placeholder="File name"
      confirm-label="Rename"
      :error="renameError"
      @confirm="submitRename"
      @cancel="cancelRename"
    />

    <ConfirmDialog
      :open="deleteTarget !== null"
      tone="danger"
      title="Delete this file?"
      message="The file stops being available anywhere it was referenced, and any link to it stops resolving. An administrator can still restore it from Trash."
      :detail="deleteTarget?.file_name"
      confirm-label="Delete"
      confirm-icon="trash-2"
      @confirm="confirmDelete"
      @cancel="deleteTarget = null"
    />
  </div>
</template>

<style scoped>
.atl-files {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 20px 24px;
}

.atl-files-filters {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 16px;
}

.atl-files-search {
  flex: 1;
  min-width: 0;
  height: 28px;
  padding: 0 8px;
  background: var(--c-input);
  border: 1px solid var(--c-border);
  color: var(--c-foreground);
  font-size: var(--fs-sm);
  font-family: inherit;
  outline: none;
}

.atl-files-search:focus {
  border-color: var(--c-primary);
}

.atl-files-name {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: 1;
  min-width: 0;
  font-size: var(--fs-sm);
  color: var(--c-foreground);
}

.atl-files-owner {
  display: flex;
  align-items: center;
  gap: 6px;
  max-width: 100%;
  border: none;
  background: transparent;
  padding: 0;
  cursor: pointer;
  font-family: inherit;
  font-size: var(--fs-sm);
  color: var(--c-muted);
}

.atl-files-owner:hover {
  color: var(--c-primary);
  text-decoration: underline;
}

.atl-files-actor {
  display: flex;
  align-items: center;
  gap: 6px;
  flex: 0 0 160px;
  min-width: 0;
  font-size: var(--fs-sm);
  color: var(--c-muted);
}

.atl-files-meta {
  font-size: var(--fs-sm);
  color: var(--c-muted);
}

.atl-files-detail {
  display: grid;
  grid-template-columns: 120px 1fr;
  gap: 6px 12px;
  margin: 0;
  font-size: var(--fs-sm);
}

.atl-files-detail dt {
  color: var(--c-muted);
}

.atl-files-detail dd {
  margin: 0;
  color: var(--c-foreground);
  overflow-wrap: anywhere;
}
</style>
