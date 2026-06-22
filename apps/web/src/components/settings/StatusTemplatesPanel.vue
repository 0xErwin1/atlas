<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue';
import Btn from '@/components/ui/Btn.vue';
import ColorPicker from '@/components/ui/ColorPicker.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import Dropdown, { type DropdownOption } from '@/components/ui/Dropdown.vue';
import Icon from '@/components/ui/Icon.vue';
import { defaultSwatchId, swatchById } from '@/lib/swatches';
import { useBoardsStore } from '@/stores/boards';
import { type StatusTemplateDto, useStatusTemplatesStore } from '@/stores/statusTemplates';
import { useUiStore } from '@/stores/ui';
import { useWorkspaceStore } from '@/stores/workspace';

/**
 * Workspace > Default statuses. Manages the workspace-level status templates new
 * boards are seeded from. The list mirrors the per-board StatusesPanel (add,
 * fold-into-edit name + color, reorder, delete) but has NO board picker because
 * templates are workspace-scoped, not per-board. A separate "Apply to a board"
 * affordance copies the templates into an existing board's columns.
 */

const workspace = useWorkspaceStore();
const templatesStore = useStatusTemplatesStore();
const boards = useBoardsStore();
const ui = useUiStore();

const ws = computed(() => workspace.activeWorkspaceSlug);

const adding = ref(false);
const newName = ref('');

const editingId = ref<string | null>(null);
const draftName = ref('');
const draftColor = ref('');

const deleteTargetId = ref<string | null>(null);
const deleteTargetName = computed(
  () => templatesStore.templates.find((t) => t.id === deleteTargetId.value)?.name ?? '',
);

const selectedBoardId = ref<string>('');
const applying = ref(false);

/** Every board in the workspace, grouped by project, for the apply-to-board picker. */
const boardOptions = computed<DropdownOption[]>(() =>
  workspace.projects.flatMap((project) =>
    boards.boardsFor(project.slug).map((board) => ({
      value: board.id,
      label: `${project.name} · ${board.name}`,
    })),
  ),
);

function swatchIdFor(template: StatusTemplateDto): string {
  return template.color ?? defaultSwatchId(`status-template:${template.id}`);
}

function swatchFg(template: StatusTemplateDto): string {
  return swatchById(swatchIdFor(template)).fg;
}

async function loadAll(): Promise<void> {
  const slug = ws.value;
  if (slug === null) return;

  await templatesStore.load(slug);
  await workspace.loadProjects(slug);
  await Promise.all(workspace.projects.map((p) => boards.loadBoardsForProject(slug, p.slug)));
}

watch(ws, () => {
  selectedBoardId.value = '';
  cancelEdit();
  void loadAll();
});

onMounted(loadAll);

async function addTemplate(): Promise<void> {
  const slug = ws.value;
  const name = newName.value.trim();
  if (slug === null || name === '') return;

  const created = await templatesStore.create(slug, name);
  if (created) {
    newName.value = '';
    adding.value = false;
    ui.showBanner('Status added', 'success');
  } else if (templatesStore.error !== null) {
    ui.showBanner(templatesStore.error, 'error');
  }
}

function startEdit(template: StatusTemplateDto): void {
  editingId.value = template.id;
  draftName.value = template.name;
  draftColor.value = swatchIdFor(template);
}

function cancelEdit(): void {
  editingId.value = null;
  draftName.value = '';
  draftColor.value = '';
}

/**
 * Persists the name and color edited together in the row's edit mode. Sends both
 * in a single PATCH; only the changed fields are included so an untouched name
 * or color is left as-is on the server.
 */
async function saveEdit(template: StatusTemplateDto): Promise<void> {
  const slug = ws.value;
  if (slug === null) {
    cancelEdit();
    return;
  }

  const nextName = draftName.value.trim();
  const patch: { name?: string; color?: string } = {};
  if (nextName !== '' && nextName !== template.name) patch.name = nextName;
  if (draftColor.value !== swatchIdFor(template)) patch.color = draftColor.value;

  if (patch.name === undefined && patch.color === undefined) {
    cancelEdit();
    return;
  }

  const ok = await templatesStore.update(slug, template.id, patch);
  if (ok) {
    ui.showBanner('Status updated', 'success');
    cancelEdit();
  } else if (templatesStore.error !== null) {
    ui.showBanner(templatesStore.error, 'error');
  }
}

/**
 * Reorders a template one slot up or down by requesting a fractional position
 * between the new neighbours. `before` is the key the template will follow and
 * `after` the key it will precede (null at the list edges).
 */
async function move(template: StatusTemplateDto, direction: -1 | 1): Promise<void> {
  const slug = ws.value;
  if (slug === null) return;

  const list = templatesStore.templates;
  const index = list.findIndex((t) => t.id === template.id);
  const target = index + direction;
  if (index === -1 || target < 0 || target >= list.length) return;

  const lower = direction === -1 ? list[target - 1] : list[target];
  const upper = direction === -1 ? list[target] : list[target + 1];

  const ok = await templatesStore.move(slug, template.id, {
    before: lower?.position_key ?? null,
    after: upper?.position_key ?? null,
  });
  if (!ok && templatesStore.error !== null) ui.showBanner(templatesStore.error, 'error');
}

async function confirmDelete(): Promise<void> {
  const slug = ws.value;
  const id = deleteTargetId.value;
  deleteTargetId.value = null;
  if (slug === null || id === null) return;

  const ok = await templatesStore.remove(slug, id);
  if (ok) ui.showBanner('Status deleted', 'success');
  else if (templatesStore.error !== null) ui.showBanner(templatesStore.error, 'error');
}

async function applyToBoard(): Promise<void> {
  const slug = ws.value;
  if (slug === null || selectedBoardId.value === '') return;

  const boardLabel = boardOptions.value.find((o) => o.value === selectedBoardId.value)?.label ?? 'board';

  applying.value = true;
  const ok = await templatesStore.applyToBoard(slug, selectedBoardId.value);
  applying.value = false;

  if (ok) ui.showBanner(`Statuses applied to ${boardLabel}`, 'success');
  else if (templatesStore.error !== null) ui.showBanner(templatesStore.error, 'error');
}
</script>

<template>
  <div>
    <div class="atl-panel-head">
      <div class="atl-panel-title">Default statuses</div>
      <div class="atl-panel-sub">Default statuses new boards start with; apply them to an existing board below.</div>
    </div>

    <div class="atl-statuses-list">
      <div
        v-for="(template, index) in templatesStore.templates"
        :key="template.id"
        class="atl-status-row"
        :class="{ editing: editingId === template.id }"
      >
        <template v-if="editingId === template.id">
          <div class="atl-edit-line">
            <span class="atl-dot" :style="{ backgroundColor: swatchById(draftColor).fg }" />
            <input
              v-model="draftName"
              type="text"
              class="atl-status-rename"
              @keydown.enter="saveEdit(template)"
              @keydown.esc="cancelEdit"
            />
            <span class="flex-1" />
            <Btn variant="primary" @click="saveEdit(template)">Save</Btn>
            <button type="button" class="atl-rowact" @click="cancelEdit">Cancel</button>
          </div>

          <ColorPicker
            class="atl-edit-picker"
            :selected="draftColor"
            @select="(id) => { draftColor = id; }"
          />
        </template>

        <template v-else>
          <span class="atl-dot" :style="{ backgroundColor: swatchFg(template) }" />
          <span class="atl-status-name">{{ template.name }}</span>
          <span class="flex-1" />
          <button
            type="button"
            class="atl-rowact icon"
            title="Move up"
            :disabled="index === 0"
            @click="move(template, -1)"
          >
            <Icon name="chevron-up" :size="14" />
          </button>
          <button
            type="button"
            class="atl-rowact icon"
            title="Move down"
            :disabled="index === templatesStore.templates.length - 1"
            @click="move(template, 1)"
          >
            <Icon name="chevron-down" :size="14" />
          </button>
          <button type="button" class="atl-rowact icon" title="Edit name & color" @click="startEdit(template)">
            <Icon name="pencil" :size="13" />
          </button>
          <button
            type="button"
            class="atl-rowact icon danger"
            title="Delete status"
            @click="deleteTargetId = template.id"
          >
            <Icon name="trash" :size="13" />
          </button>
        </template>
      </div>
    </div>

    <div v-if="templatesStore.templates.length === 0 && !adding" class="atl-statuses-empty">
      No default statuses yet. Add one below.
    </div>

    <div v-if="adding" class="atl-status-add">
      <input
        v-model="newName"
        type="text"
        placeholder="Status name…"
        class="atl-status-rename"
        @keydown.enter="addTemplate"
        @keydown.esc="adding = false"
      />
      <Btn variant="primary" :disabled="newName.trim() === ''" @click="addTemplate">Add</Btn>
      <button type="button" class="atl-rowact" @click="adding = false">Cancel</button>
    </div>
    <Btn v-else variant="secondary" style="margin-top: 12px;" @click="adding = true">
      <Icon name="plus" :size="14" />Add status
    </Btn>

    <div class="atl-apply-section">
      <div class="atl-apply-title">Apply to a board</div>
      <div class="atl-apply-sub">Adds any missing default statuses to an existing board. Existing statuses are left untouched.</div>
      <div class="atl-apply-row">
        <Dropdown
          :model-value="selectedBoardId"
          :options="boardOptions"
          placeholder="Pick a board…"
          icon="kanban"
          @update:model-value="(id) => { selectedBoardId = id; }"
        />
        <Btn
          variant="primary"
          :disabled="selectedBoardId === '' || applying || templatesStore.templates.length === 0"
          @click="applyToBoard"
        >
          Apply
        </Btn>
      </div>
    </div>

    <ConfirmDialog
      :open="deleteTargetId !== null"
      tone="danger"
      title="Delete this default status?"
      message="It is removed from the workspace defaults. Boards already using it keep their column."
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

.atl-statuses-empty {
  font-size: 13px;
  color: var(--c-muted);
  padding: 8px 2px;
}

.atl-statuses-list:empty {
  display: none;
}

.atl-statuses-list {
  border: 1px solid var(--c-border);
  border-radius: 4px;
  overflow: hidden;
  max-width: 560px;
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

.atl-apply-section {
  margin-top: 28px;
  padding-top: 20px;
  border-top: 1px solid var(--c-border);
  max-width: 560px;
}

.atl-apply-title {
  font-size: 13px;
  font-weight: var(--fw-semibold);
  color: var(--c-foreground);
}

.atl-apply-sub {
  font-size: 12px;
  color: var(--c-muted);
  margin-top: 3px;
}

.atl-apply-row {
  display: flex;
  align-items: center;
  gap: 10px;
  margin-top: 12px;
}
</style>
