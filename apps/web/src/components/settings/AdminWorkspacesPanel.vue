<script setup lang="ts">
import { onMounted, ref } from 'vue';
import Btn from '@/components/ui/Btn.vue';
import Icon from '@/components/ui/Icon.vue';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore, type WorkspaceDto } from '@/stores/workspace';

/**
 * Administration > Workspaces (root-only). Lists every workspace in the system
 * and renames each in place via the same workspace rename endpoint. No delete:
 * removing a workspace is out of scope and far more destructive than a rename.
 */

const workspace = useWorkspaceStore();
const ui = useUiStore();

const editingSlug = ref<string | null>(null);
const draftName = ref('');
const saving = ref(false);

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
</script>

<template>
  <div>
    <div class="atl-panel-head">
      <div class="atl-panel-title">Workspaces</div>
      <div class="atl-panel-sub">Every workspace in the system — rename any of them</div>
    </div>

    <div class="atl-ws-table">
      <div class="atl-ws-head">
        <div style="flex: 2;">Name</div>
        <div style="flex: 1;">Slug</div>
        <div style="flex: 0 0 96px;"></div>
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
        <div style="flex: 0 0 96px; display: flex; justify-content: flex-end; gap: 6px;">
          <template v-if="editingSlug === ws.slug">
            <Btn variant="primary" :disabled="saving" @click="saveEdit(ws)">Save</Btn>
            <button type="button" class="atl-rowact" @click="cancelEdit">Cancel</button>
          </template>
          <button v-else type="button" class="atl-rowact" title="Rename" @click="startEdit(ws)">
            <Icon name="pencil" :size="13" />
          </button>
        </div>
      </div>

      <div
        v-if="workspace.adminWorkspaces.length === 0"
        style="padding: 14px 12px; font-size: 13px; color: var(--c-muted);"
      >
        No workspaces.
      </div>
    </div>
  </div>
</template>

<style scoped>
.atl-panel-head {
  margin-bottom: 16px;
}

.atl-panel-title {
  font-size: 15px;
  font-weight: var(--fw-bold);
  color: var(--c-foreground);
}

.atl-panel-sub {
  font-size: 12px;
  color: var(--c-muted);
  margin-top: 3px;
}

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

.atl-rowact {
  display: inline-flex;
  align-items: center;
  height: 24px;
  padding: 0 8px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  background: transparent;
  color: var(--c-foreground);
  cursor: pointer;
  font-size: 12px;
}

.atl-rowact:hover {
  background: var(--c-raised);
}
</style>
