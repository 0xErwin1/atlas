<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import PanelHeader from '@/components/settings/PanelHeader.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import Icon from '@/components/ui/Icon.vue';
import { formatDate } from '@/lib/format';
import { type TrashItem, type TrashKind, useTrashStore } from '@/stores/trash';
import { useWorkspaceStore } from '@/stores/workspace';

const trash = useTrashStore();
const workspace = useWorkspaceStore();
const selectedWorkspace = ref('');
const selectedKind = ref<TrashKind | ''>('');
const purgeTarget = ref<TrashItem | null>(null);
const operation = ref<ReturnType<typeof trash.purge> extends Promise<infer T> ? T : null>(null);
let pollTimer: ReturnType<typeof setTimeout> | null = null;

const kinds: Array<{ value: TrashKind | ''; label: string }> = [
  { value: '', label: 'All types' },
  { value: 'project', label: 'Projects' },
  { value: 'folder', label: 'Folders' },
  { value: 'document', label: 'Documents' },
  { value: 'comment', label: 'Comments' },
  { value: 'attachment', label: 'Attachments' },
];

const confirmationText = computed(() =>
  purgeTarget.value === null ? '' : `PURGE ${purgeTarget.value.target_id}`,
);

function currentFilter() {
  return {
    ...(selectedWorkspace.value === '' ? {} : { workspaceId: selectedWorkspace.value }),
    ...(selectedKind.value === '' ? {} : { kind: selectedKind.value }),
  };
}

async function refresh(): Promise<void> {
  await trash.load(currentFilter());
}

async function restore(item: TrashItem): Promise<void> {
  await trash.restore(item);
}

async function confirmPurge(): Promise<void> {
  const target = purgeTarget.value;
  if (target === null) return;
  purgeTarget.value = null;
  operation.value = await trash.purge(target);
  schedulePoll();
}

async function poll(): Promise<void> {
  const current = operation.value;
  if (current === null || current.status === 'complete') return;
  operation.value = await trash.poll(current.operation_id);
  schedulePoll();
}

function schedulePoll(): void {
  if (pollTimer !== null) clearTimeout(pollTimer);
  if (operation.value !== null && operation.value.status !== 'complete') {
    pollTimer = setTimeout(() => void poll(), 2_000);
  }
}

onBeforeUnmount(() => {
  if (pollTimer !== null) clearTimeout(pollTimer);
});

onMounted(() => {
  void workspace.loadAdminWorkspaces();
  void refresh();
});
</script>

<template>
  <div>
    <PanelHeader title="Trash" subtitle="Recover deleted resources or permanently purge them">
      <template #actions>
        <button type="button" class="atl-trash-refresh" :disabled="trash.loading" @click="refresh">
          <Icon name="refresh-cw" :size="14" /> Refresh
        </button>
      </template>
    </PanelHeader>

    <div class="atl-trash-filters">
      <select v-model="selectedWorkspace" aria-label="Workspace filter" @change="refresh">
        <option value="">All workspaces</option>
        <option v-for="entry in workspace.adminWorkspaces" :key="entry.id" :value="entry.id">{{ entry.name }}</option>
      </select>
      <select v-model="selectedKind" aria-label="Resource type filter" @change="refresh">
        <option v-for="kind in kinds" :key="kind.value" :value="kind.value">{{ kind.label }}</option>
      </select>
    </div>

    <p v-if="trash.error" role="alert" class="atl-trash-error">{{ trash.error }}</p>
    <p v-else-if="operation" class="atl-trash-status" role="status">
      Purge {{ operation.status.replace('_', ' ') }} (attempt {{ operation.attempts }})
      <button v-if="operation.status !== 'complete'" type="button" @click="poll">Check status</button>
    </p>

    <div v-if="trash.items.length > 0" class="atl-trash-list">
      <div v-for="item in trash.items" :key="`${item.kind}:${item.target_id}`" class="atl-trash-row">
        <Icon name="trash-2" :size="15" />
        <div class="atl-trash-item"><strong>{{ item.kind }}</strong><code>{{ item.target_id }}</code><span>{{ formatDate(item.deleted_at) }}</span></div>
        <button type="button" @click="restore(item)">Restore</button>
        <button type="button" class="danger" @click="purgeTarget = item">Purge</button>
      </div>
    </div>
    <p v-else-if="!trash.loading" class="atl-trash-empty">No deleted resources match these filters.</p>
    <button v-if="trash.hasMore" type="button" class="atl-trash-more" @click="trash.loadMore">Load more</button>

    <ConfirmDialog
      :open="purgeTarget !== null"
      tone="danger"
      title="Permanently purge resource?"
      message="This permanently removes the resource and any eligible stored data. This cannot be undone."
      :detail="purgeTarget?.target_id"
      :confirmation-text="confirmationText"
      confirm-label="Purge permanently"
      confirm-icon="trash-2"
      @confirm="confirmPurge"
      @cancel="purgeTarget = null"
    />
  </div>
</template>

<style scoped>
.atl-trash-filters,.atl-trash-row { display:flex; align-items:center; gap:8px; }
.atl-trash-filters { margin-bottom:16px; }
.atl-trash-list { border:1px solid var(--c-border); border-radius:var(--r-sm); }
.atl-trash-row { padding:10px; border-top:1px solid var(--c-border); }
.atl-trash-row:first-child { border-top:0; }
.atl-trash-item { display:flex; flex:1; min-width:0; flex-direction:column; gap:2px; }
.atl-trash-item code { overflow:hidden; text-overflow:ellipsis; white-space:nowrap; color:var(--c-muted); }
.atl-trash-item span,.atl-trash-empty,.atl-trash-error,.atl-trash-status { font-size:var(--fs-sm); color:var(--c-muted); }
.atl-trash-error { color:var(--c-danger); }.danger { color:var(--c-danger); }.atl-trash-more { margin-top:10px; }
</style>
