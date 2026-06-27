<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue';
import PanelHeader from '@/components/settings/PanelHeader.vue';
import Btn from '@/components/ui/Btn.vue';
import ColorPicker from '@/components/ui/ColorPicker.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import Dropdown, { type DropdownOption } from '@/components/ui/Dropdown.vue';
import Icon from '@/components/ui/Icon.vue';
import { resolveColumnSwatchId } from '@/lib/columnColor';
import { swatchById } from '@/lib/swatches';
import { type ColumnDto, useBoardsStore } from '@/stores/boards';
import { useStatusTemplatesStore } from '@/stores/statusTemplates';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

/**
 * Workspace > Statuses. Statuses (board columns) are per-board, and boards are
 * per-project, so editing them starts with a board picker that aggregates every
 * board across the workspace's projects. After picking a board, its columns can
 * be added, renamed, reordered, recolored and deleted; color is persisted on the
 * column as a swatch id.
 */

const workspace = useWorkspaceStore();
const boards = useBoardsStore();
const templatesStore = useStatusTemplatesStore();
const ui = useUiStore();

const ws = computed(() => workspace.activeWorkspaceSlug);

const selectedBoardId = ref<string>('');
const loadingColumns = ref(false);
const applyingDefaults = ref(false);

const adding = ref(false);
const newColumnName = ref('');

const editingId = ref<string | null>(null);
const draftName = ref('');
const draftColor = ref('');

const deleteTargetId = ref<string | null>(null);

const deleteTargetName = computed(
  () => boards.columns.find((c) => c.id === deleteTargetId.value)?.name ?? '',
);

/**
 * Every board in the workspace, grouped by project for the picker. Each project's
 * boards are loaded into the boards store's per-project map; this flattens them
 * into labelled options ("Project · Board") so identically named boards on
 * different projects stay distinguishable.
 */
const boardOptions = computed<DropdownOption[]>(() =>
  workspace.projects.flatMap((project) =>
    boards.boardsFor(project.slug).map((board) => ({
      value: board.id,
      label: `${project.name} · ${board.name}`,
    })),
  ),
);

async function loadAllBoards(): Promise<void> {
  const slug = ws.value;
  if (slug === null) return;

  await workspace.loadProjects(slug);
  await Promise.all(workspace.projects.map((p) => boards.loadBoardsForProject(slug, p.slug)));
}

watch(ws, () => {
  selectedBoardId.value = '';
  void loadAllBoards();
});

onMounted(loadAllBoards);

async function onBoardSelected(boardId: string): Promise<void> {
  const slug = ws.value;
  selectedBoardId.value = boardId;
  if (slug === null || boardId === '') return;

  loadingColumns.value = true;
  await boards.loadColumns(slug, boardId);
  loadingColumns.value = false;
}

function swatchFg(column: ColumnDto): string {
  return swatchById(resolveColumnSwatchId(column)).fg;
}

async function addColumn(): Promise<void> {
  const slug = ws.value;
  const name = newColumnName.value.trim();
  if (slug === null || selectedBoardId.value === '' || name === '') return;

  const created = await boards.createColumn(slug, selectedBoardId.value, name);
  if (created) {
    newColumnName.value = '';
    adding.value = false;
    ui.showBanner('Status added', 'success');
  } else if (boards.error) {
    ui.showBanner(boards.error, 'error');
  }
}

function startRename(column: ColumnDto): void {
  editingId.value = column.id;
  draftName.value = column.name;
  draftColor.value = resolveColumnSwatchId(column);
}

function cancelRename(): void {
  editingId.value = null;
  draftName.value = '';
  draftColor.value = '';
}

/**
 * Persists the name and color edited together in the row's edit mode. Sends both
 * in a single PATCH; only the changed fields are included so an untouched name
 * or color is left as-is on the server.
 */
async function saveEdit(column: ColumnDto): Promise<void> {
  const slug = ws.value;
  if (slug === null) {
    cancelRename();
    return;
  }

  const nextName = draftName.value.trim();
  const patch: { name?: string; color?: string } = {};
  if (nextName !== '' && nextName !== column.name) patch.name = nextName;
  if (draftColor.value !== resolveColumnSwatchId(column)) patch.color = draftColor.value;

  if (patch.name === undefined && patch.color === undefined) {
    cancelRename();
    return;
  }

  const ok = await boards.updateColumn(slug, selectedBoardId.value, column.id, patch);
  if (ok) {
    ui.showBanner('Status updated', 'success');
    cancelRename();
  } else if (boards.error) {
    ui.showBanner(boards.error, 'error');
  }
}

/**
 * Reorders a column one slot up or down by requesting a fractional position
 * between the new neighbours. `before` is the key the column will follow and
 * `after` the key it will precede (null at the list edges).
 */
async function move(column: ColumnDto, direction: -1 | 1): Promise<void> {
  const slug = ws.value;
  if (slug === null) return;

  const list = boards.columns;
  const index = list.findIndex((c) => c.id === column.id);
  const target = index + direction;
  if (index === -1 || target < 0 || target >= list.length) return;

  const lower = direction === -1 ? list[target - 1] : list[target];
  const upper = direction === -1 ? list[target] : list[target + 1];

  const ok = await boards.moveColumn(slug, selectedBoardId.value, column.id, {
    before: lower?.position_key ?? null,
    after: upper?.position_key ?? null,
  });
  if (!ok && boards.error) ui.showBanner(boards.error, 'error');
}

/**
 * Copies the workspace status templates into the selected board (adding only the
 * statuses it does not already have by name), then reloads its columns so the new
 * ones appear immediately.
 */
async function applyDefaults(): Promise<void> {
  const slug = ws.value;
  if (slug === null || selectedBoardId.value === '') return;

  applyingDefaults.value = true;
  const ok = await templatesStore.applyToBoard(slug, selectedBoardId.value);
  if (ok) await boards.loadColumns(slug, selectedBoardId.value);
  applyingDefaults.value = false;

  if (ok) ui.showBanner('Workspace defaults applied', 'success');
  else if (templatesStore.error !== null) ui.showBanner(templatesStore.error, 'error');
}

async function confirmDelete(): Promise<void> {
  const slug = ws.value;
  const id = deleteTargetId.value;
  deleteTargetId.value = null;
  if (slug === null || id === null) return;

  const ok = await boards.deleteColumn(slug, selectedBoardId.value, id);
  if (ok) ui.showBanner('Status deleted', 'success');
  else if (boards.error) ui.showBanner(boards.error, 'error');
}
</script>

<template>
  <div>
    <PanelHeader
      title="Statuses"
      subtitle="Edit the columns of a board — add, edit name &amp; color, reorder or delete"
    />

    <div class="atl-board-pick">
      <span class="atl-field-label">Board</span>
      <Dropdown
        :model-value="selectedBoardId"
        :options="boardOptions"
        placeholder="Pick a board…"
        icon="kanban"
        @update:model-value="onBoardSelected"
      />
    </div>

    <div
      v-if="selectedBoardId === ''"
      class="atl-statuses-empty"
    >
      Pick a board above to edit its statuses.
    </div>

    <div v-else-if="loadingColumns" class="atl-statuses-empty">Loading…</div>

    <div v-else>
      <div class="atl-statuses-list">
        <div
          v-for="(column, index) in boards.columns"
          :key="column.id"
          class="atl-status-row"
          :class="{ editing: editingId === column.id }"
        >
          <template v-if="editingId === column.id">
            <div class="atl-edit-line">
              <span class="atl-dot" :style="{ backgroundColor: swatchById(draftColor).fg }" />
              <input
                v-model="draftName"
                type="text"
                class="atl-status-rename"
                @keydown.enter="saveEdit(column)"
                @keydown.esc="cancelRename"
              />
              <span class="flex-1" />
              <Btn variant="primary" @click="saveEdit(column)">Save</Btn>
              <button type="button" class="atl-rowact" @click="cancelRename">Cancel</button>
            </div>

            <ColorPicker
              class="atl-edit-picker"
              :selected="draftColor"
              @select="(id) => { draftColor = id; }"
            />
          </template>

          <template v-else>
            <span class="atl-dot" :style="{ backgroundColor: swatchFg(column) }" />
            <span class="atl-status-name">{{ column.name }}</span>
            <span class="flex-1" />
            <button
              type="button"
              class="atl-rowact icon"
              title="Move up"
              :disabled="index === 0"
              @click="move(column, -1)"
            >
              <Icon name="chevron-up" :size="14" />
            </button>
            <button
              type="button"
              class="atl-rowact icon"
              title="Move down"
              :disabled="index === boards.columns.length - 1"
              @click="move(column, 1)"
            >
              <Icon name="chevron-down" :size="14" />
            </button>
            <button type="button" class="atl-rowact icon" title="Edit name & color" @click="startRename(column)">
              <Icon name="pencil" :size="13" />
            </button>
            <button
              type="button"
              class="atl-rowact icon danger"
              title="Delete status"
              @click="deleteTargetId = column.id"
            >
              <Icon name="trash" :size="13" />
            </button>
          </template>
        </div>
      </div>

      <div v-if="adding" class="atl-status-add">
        <input
          v-model="newColumnName"
          type="text"
          placeholder="Status name…"
          class="atl-status-rename"
          @keydown.enter="addColumn"
          @keydown.esc="adding = false"
        />
        <Btn variant="primary" :disabled="newColumnName.trim() === ''" @click="addColumn">Add</Btn>
        <button type="button" class="atl-rowact" @click="adding = false">Cancel</button>
      </div>
      <div v-if="!adding" class="atl-status-actions">
        <Btn variant="secondary" @click="adding = true">
          <Icon name="plus" :size="14" />Add status
        </Btn>
        <Btn variant="ghost" :disabled="applyingDefaults" @click="applyDefaults">
          <Icon name="kanban" :size="14" />Apply workspace defaults
        </Btn>
      </div>
    </div>

    <ConfirmDialog
      :open="deleteTargetId !== null"
      tone="danger"
      title="Delete this status?"
      message="The column is removed from the board. It must have no tasks; move or delete its tasks first."
      :detail="deleteTargetName"
      detail-icon="kanban"
      confirm-label="Delete status"
      confirm-icon="trash"
      @confirm="confirmDelete"
      @cancel="deleteTargetId = null"
    />
  </div>
</template>

<style scoped>
.atl-board-pick {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-bottom: 18px;
}

.atl-field-label {
  font-size: 12px;
  font-weight: var(--fw-semibold);
  color: var(--c-muted);
}

.atl-statuses-empty {
  font-size: 13px;
  color: var(--c-muted);
  padding: 8px 2px;
}

.atl-statuses-list {
  border: 1px solid var(--c-border);
  border-radius: 4px;
  overflow: hidden;
}

.atl-status-row {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 48px;
  padding: 0 12px;
  border-top: 1px solid var(--c-border);
}

.atl-status-row:first-child {
  border-top: none;
}

.atl-status-row.editing {
  height: auto;
  flex-direction: column;
  align-items: stretch;
  gap: 10px;
  padding-top: 12px;
  padding-bottom: 14px;
}

.atl-edit-line {
  display: flex;
  align-items: center;
  gap: 8px;
}

.atl-edit-picker {
  align-self: flex-start;
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  background: var(--c-raised);
}

.atl-dot {
  flex: none;
  width: 9px;
  height: 9px;
  border-radius: var(--r-full);
}

.atl-status-name {
  font-size: 13px;
  font-weight: var(--fw-semibold);
  color: var(--c-foreground);
}

.atl-status-rename {
  height: var(--h-button);
  width: 220px;
  padding: 0 10px;
  background: var(--c-raised);
  border: 1px solid var(--c-primary);
  border-radius: var(--r-md);
  font-size: 13px;
  color: var(--c-foreground);
  outline: none;
}

.atl-status-add {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 12px;
}

.atl-status-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-top: 12px;
}

.atl-rowact {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  height: 26px;
  padding: 0 8px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  background: transparent;
  color: var(--c-foreground);
  cursor: pointer;
  font-size: 12px;
}

.atl-rowact.icon {
  width: 28px;
  padding: 0;
}

.atl-rowact:hover:enabled {
  background: var(--c-raised);
}

.atl-rowact:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.atl-rowact.danger {
  color: var(--c-danger);
}
</style>
