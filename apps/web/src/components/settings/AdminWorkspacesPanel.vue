<script setup lang="ts">
import { onMounted, ref } from 'vue';
import PanelHeader from '@/components/settings/PanelHeader.vue';
import RowAction from '@/components/settings/RowAction.vue';
import Btn from '@/components/ui/Btn.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import Icon from '@/components/ui/Icon.vue';
import PromptDialog from '@/components/ui/PromptDialog.vue';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore, type WorkspaceDto } from '@/stores/workspace';

/**
 * Administration > Workspaces (root/system-admin only). Lists every workspace in
 * the system and offers full lifecycle control: create a new workspace, rename it,
 * change its slug, and soft-delete it. Slug changes and deletes go through the
 * dedicated admin endpoints; the rename reuses the shared workspace rename path.
 */

const workspace = useWorkspaceStore();
const ui = useUiStore();

const editingSlug = ref<string | null>(null);
const draftName = ref('');
const saving = ref(false);

const createOpen = ref(false);
const slugTarget = ref<WorkspaceDto | null>(null);
const deleteTarget = ref<WorkspaceDto | null>(null);

onMounted(() => workspace.loadAdminWorkspaces());

function startEdit(ws: WorkspaceDto): void {
  editingSlug.value = ws.slug;
  draftName.value = ws.name;
}

function cancelEdit(): void {
  editingSlug.value = null;
  draftName.value = '';
}

async function saveEdit(ws: WorkspaceDto): Promise<void> {
  const next = draftName.value.trim();
  if (next === '' || next === ws.name) {
    cancelEdit();
    return;
  }

  saving.value = true;
  const ok = await workspace.renameWorkspace(ws.slug, next);
  saving.value = false;

  if (ok) {
    ui.showBanner('Workspace renamed', 'success');
    cancelEdit();
  } else if (workspace.error) {
    ui.showBanner(workspace.error, 'error');
  }
}

async function submitCreate(name: string): Promise<void> {
  const trimmed = name.trim();
  if (trimmed === '') return;

  const slug = await workspace.createWorkspace(trimmed);
  createOpen.value = false;

  if (slug !== null) {
    await workspace.loadAdminWorkspaces();
    ui.showBanner('Workspace created', 'success');
  } else if (workspace.error) {
    ui.showBanner(workspace.error, 'error');
  }
}

async function submitSlug(slug: string): Promise<void> {
  const target = slugTarget.value;
  if (target === null) return;

  const next = slug.trim();
  slugTarget.value = null;

  if (next === '' || next === target.slug) return;

  const ok = await workspace.updateWorkspaceSlug(target.slug, next);
  if (ok) {
    ui.showBanner('Workspace slug updated', 'success');
  } else if (workspace.error) {
    ui.showBanner(workspace.error, 'error');
  }
}

async function confirmDelete(): Promise<void> {
  const target = deleteTarget.value;
  if (target === null) return;

  deleteTarget.value = null;

  const ok = await workspace.deleteWorkspace(target.slug);
  if (!ok) {
    if (workspace.error) ui.showBanner(workspace.error, 'error');
    return;
  }

  ui.showBanner(`Workspace "${target.name}" deleted`, 'success');
}
</script>

<template>
  <div>
    <PanelHeader title="Workspaces" subtitle="Every workspace in the system — create, rename, re-slug, or delete">
      <template #actions>
        <Btn variant="primary" @click="createOpen = true">
          <Icon name="plus" :size="14" />
          New workspace
        </Btn>
      </template>
    </PanelHeader>

    <div class="atl-ws-table">
      <div class="atl-ws-head">
        <div style="flex: 2;">Name</div>
        <div style="flex: 1;">Slug</div>
        <div style="flex: 0 0 132px;"></div>
      </div>

      <div v-for="ws in workspace.adminWorkspaces" :key="ws.id" class="atl-ws-row">
        <div style="flex: 2; min-width: 0;">
          <input
            v-if="editingSlug === ws.slug"
            v-model="draftName"
            type="text"
            class="atl-ws-input"
            @keydown.enter="saveEdit(ws)"
            @keydown.esc="cancelEdit"
          />
          <span v-else class="atl-ws-name">{{ ws.name }}</span>
        </div>
        <div style="flex: 1; min-width: 0;">
          <code class="atl-ws-slug">{{ ws.slug }}</code>
        </div>
        <div style="flex: 0 0 132px; display: flex; justify-content: flex-end; gap: 6px;">
          <template v-if="editingSlug === ws.slug">
            <Btn variant="primary" :disabled="saving" @click="saveEdit(ws)">Save</Btn>
            <RowAction @click="cancelEdit">Cancel</RowAction>
          </template>
          <template v-else>
            <RowAction title="Rename" @click="startEdit(ws)">
              <Icon name="pencil" :size="13" />
            </RowAction>
            <RowAction title="Edit slug" @click="slugTarget = ws">
              <Icon name="link" :size="13" />
            </RowAction>
            <RowAction title="Delete" @click="deleteTarget = ws">
              <Icon name="trash-2" :size="13" />
            </RowAction>
          </template>
        </div>
      </div>

      <div
        v-if="workspace.adminWorkspaces.length === 0"
        style="padding: 14px 12px; font-size: 13px; color: var(--c-muted);"
      >
        No workspaces.
      </div>
    </div>

    <PromptDialog
      :open="createOpen"
      title="New workspace"
      placeholder="Workspace name"
      confirm-label="Create"
      @confirm="submitCreate"
      @cancel="createOpen = false"
    />

    <PromptDialog
      :open="slugTarget !== null"
      title="Edit workspace slug"
      :initial="slugTarget?.slug ?? ''"
      placeholder="workspace-slug"
      confirm-label="Save"
      @confirm="submitSlug"
      @cancel="slugTarget = null"
    />

    <ConfirmDialog
      :open="deleteTarget !== null"
      tone="danger"
      title="Delete workspace?"
      message="The workspace and all of its content will be hidden from everyone. This can only be undone by an operator."
      :detail="deleteTarget?.name"
      detail-icon="building-2"
      confirm-label="Delete workspace"
      confirm-icon="trash-2"
      @confirm="confirmDelete"
      @cancel="deleteTarget = null"
    />
  </div>
</template>

<style scoped>
.atl-ws-table {
  border: 1px solid var(--c-border);
  border-radius: 4px;
  overflow: hidden;
}

.atl-ws-head {
  display: flex;
  align-items: center;
  height: 28px;
  padding: 0 12px;
  font-size: 10px;
  font-weight: var(--fw-semibold);
  letter-spacing: 0.05em;
  text-transform: uppercase;
  color: var(--c-muted);
}

.atl-ws-row {
  display: flex;
  align-items: center;
  height: 46px;
  padding: 0 12px;
  border-top: 1px solid var(--c-border);
}

.atl-ws-name {
  font-size: 13px;
  font-weight: var(--fw-semibold);
  color: var(--c-foreground);
}

.atl-ws-slug {
  font-size: 12px;
  font-family: var(--font-mono);
  color: var(--c-muted);
}

.atl-ws-input {
  width: 100%;
  height: 30px;
  padding: 0 9px;
  background: var(--c-raised);
  border: 1px solid var(--c-primary);
  border-radius: var(--r-md);
  font-size: 13px;
  color: var(--c-foreground);
  outline: none;
}

</style>
