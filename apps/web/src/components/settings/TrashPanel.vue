<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { getResourceCachePrincipal, resourceCache } from '@/cache/cacheRuntime';
import ExpandableRow from '@/components/settings/ExpandableRow.vue';
import PanelHeader from '@/components/settings/PanelHeader.vue';
import RowAction from '@/components/settings/RowAction.vue';
import SettingsTable from '@/components/settings/SettingsTable.vue';
import EmptyState from '@/components/states/EmptyState.vue';
import Btn from '@/components/ui/Btn.vue';
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
const expandedTarget = ref<string | null>(null);
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
  if (!(await trash.restore(item))) return;

  const activeWorkspace = workspace.activeWorkspaceSlug;
  if (activeWorkspace === null) return;

  await workspace.loadProjects(activeWorkspace);
  if (workspace.projectsError !== null) {
    trash.error = workspace.projectsError;
    return;
  }

  const workspaceId = workspace.workspaceIdForSlug(activeWorkspace);
  if (workspaceId === null) return;

  const restoredProject =
    item.kind === 'project' ? workspace.projects.find((project) => project.id === item.target_id) : undefined;
  try {
    const invalidated = await resourceCache.purgeTags(
      [
        `workspace:${workspaceId}`,
        ...workspace.projects.map((project) => `project:${project.slug}`),
        ...(restoredProject === undefined ? [] : [`project:${restoredProject.slug}`]),
        ...(item.kind === 'document' ? [`document:${item.target_id}`] : []),
        ...(item.kind === 'comment' ? [`comment:${item.target_id}`] : []),
        ...(item.kind === 'attachment' ? [`attachment:${item.target_id}`] : []),
        ...(item.kind === 'folder' ? [`folder:${item.target_id}`] : []),
      ],
      getResourceCachePrincipal(),
      workspaceId,
    );
    if (!invalidated) trash.error = 'Restored resource, but cached resources could not be refreshed.';
  } catch {
    trash.error = 'Restored resource, but cached resources could not be refreshed.';
  }
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
  const next = await trash.poll(current.operation_id);
  if (next !== null) operation.value = next;
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
        <Btn :disabled="trash.loading" @click="refresh">
          <Icon name="refresh-cw" :size="14" /> Refresh
        </Btn>
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

    <p v-if="trash.error" role="alert" class="atl-trash-message atl-trash-message--error">{{ trash.error }}</p>
    <p v-else-if="operation" class="atl-trash-message" role="status">
      Purge {{ operation.status.replace('_', ' ') }} (attempt {{ operation.attempts }})
      <Btn v-if="operation.status !== 'complete'" @click="poll">Check status</Btn>
    </p>

    <SettingsTable v-if="trash.items.length > 0">
      <template #head>
        <div style="flex: 0 0 110px;">Type</div>
        <div style="flex: 1;">Resource</div>
        <div style="flex: 0 0 132px;">Deleted</div>
        <div style="flex: 0 0 150px;"></div>
      </template>
      <ExpandableRow
        v-for="item in trash.items"
        :key="`${item.kind}:${item.target_id}`"
        :expanded="expandedTarget === `${item.kind}:${item.target_id}`"
        style="--erow-actions-basis: 150px; min-height: 46px;"
        @toggle="expandedTarget = expandedTarget === `${item.kind}:${item.target_id}` ? null : `${item.kind}:${item.target_id}`"
      >
        <template #summary>
          <div style="display: flex; flex: 0 0 110px; align-items: center; gap: 6px;"><Icon name="trash-2" :size="14" />{{ item.kind }}</div>
          <code class="atl-trash-target">{{ item.target_id }}</code>
          <span style="flex: 0 0 132px; font-size: var(--fs-sm); color: var(--c-muted);">{{ formatDate(item.deleted_at) }}</span>
        </template>
        <template #actions>
          <RowAction title="Restore" @click="restore(item)"><Icon name="rotate-ccw" :size="13" /></RowAction>
          <RowAction title="Purge permanently" @click="purgeTarget = item"><Icon name="trash-2" :size="13" /></RowAction>
        </template>
        <template #panel>
          <code class="atl-trash-target">{{ item.target_id }}</code>
        </template>
      </ExpandableRow>
    </SettingsTable>
    <EmptyState
      v-else-if="!trash.loading"
      compact
      icon="trash-2"
      title="No deleted resources match these filters."
    />
    <Btn v-if="trash.hasMore" style="margin-top: 10px;" @click="trash.loadMore">Load more</Btn>

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
  .atl-trash-filters { display:flex; align-items:center; gap:8px; }
  .atl-trash-filters { margin-bottom:16px; }
  .atl-trash-target { flex: 1; min-width: 0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; color:var(--c-muted); }
  .atl-trash-message { display:flex; align-items:center; gap:8px; font-size:var(--fs-sm); color:var(--c-muted); }
  .atl-trash-message--error { color:var(--c-danger); }
</style>
