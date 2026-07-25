<script setup lang="ts">
import { computed, ref, watch } from 'vue';
import RowAction from '@/components/settings/RowAction.vue';
import Btn from '@/components/ui/Btn.vue';
import ColorPicker from '@/components/ui/ColorPicker.vue';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import Icon from '@/components/ui/Icon.vue';
import { defaultSwatchId, swatchById } from '@/lib/swatches';

/**
 * Editable status-template list shared by the workspace and Atlas-wide default
 * status panels. Owns the row interaction state (add, fold-into-edit name +
 * color, reorder, delete confirmation) and emits intents; persistence, banners
 * and error handling stay with the owning panel and its store.
 */

/** Row shape both surfaces share — `PlatformStatusTemplateDto` is this minus nothing. */
export interface StatusTemplateRow {
  id: string;
  name: string;
  color?: string | null;
  position_key: string;
}

/** Reorder anchors: the position keys the row lands between (null at the edges). */
export interface StatusTemplatePlacement {
  before: string | null;
  after: string | null;
}

const props = defineProps<{
  templates: StatusTemplateRow[];
  emptyLabel: string;
  deleteTitle: string;
  deleteMessage: string;
}>();

const emit = defineEmits<{
  create: [name: string];
  update: [id: string, patch: { name?: string; color?: string }];
  move: [id: string, placement: StatusTemplatePlacement];
  remove: [id: string];
}>();

const adding = ref(false);
const newName = ref('');

const editingId = ref<string | null>(null);
const draftName = ref('');
const draftColor = ref('');

const deleteTargetId = ref<string | null>(null);
const deleteTargetName = computed(
  () => props.templates.find((t) => t.id === deleteTargetId.value)?.name ?? '',
);

function swatchIdFor(template: StatusTemplateRow): string {
  return template.color ?? defaultSwatchId(`status-template:${template.id}`);
}

function swatchFg(template: StatusTemplateRow): string {
  return swatchById(swatchIdFor(template)).fg;
}

function cancelEdit(): void {
  editingId.value = null;
  draftName.value = '';
  draftColor.value = '';
}

function startEdit(template: StatusTemplateRow): void {
  editingId.value = template.id;
  draftName.value = template.name;
  draftColor.value = swatchIdFor(template);
}

function addTemplate(): void {
  const name = newName.value.trim();
  if (name === '') return;

  newName.value = '';
  adding.value = false;
  emit('create', name);
}

/**
 * Emits the name and color edited together in the row's edit mode; only the
 * changed fields travel, so an untouched name or color is left as-is.
 */
function saveEdit(template: StatusTemplateRow): void {
  const nextName = draftName.value.trim();
  const patch: { name?: string; color?: string } = {};
  if (nextName !== '' && nextName !== template.name) patch.name = nextName;
  if (draftColor.value !== swatchIdFor(template)) patch.color = draftColor.value;

  cancelEdit();
  if (patch.name === undefined && patch.color === undefined) return;

  emit('update', template.id, patch);
}

/**
 * Reorders a template one slot up or down: `before` is the key the template
 * will follow and `after` the key it will precede (null at the list edges).
 */
function move(template: StatusTemplateRow, direction: -1 | 1): void {
  const list = props.templates;
  const index = list.findIndex((t) => t.id === template.id);
  const target = index + direction;
  if (index === -1 || target < 0 || target >= list.length) return;

  const lower = direction === -1 ? list[target - 1] : list[target];
  const upper = direction === -1 ? list[target] : list[target + 1];

  emit('move', template.id, {
    before: lower?.position_key ?? null,
    after: upper?.position_key ?? null,
  });
}

function confirmDelete(): void {
  const id = deleteTargetId.value;
  deleteTargetId.value = null;
  if (id === null) return;

  emit('remove', id);
}

watch(
  () => props.templates,
  () => {
    if (editingId.value !== null && !props.templates.some((t) => t.id === editingId.value)) {
      cancelEdit();
    }
  },
);

defineExpose({ cancelEdit });
</script>

<template>
  <div>
    <div class="atl-statuses-list">
      <div
        v-for="(template, index) in templates"
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
            <RowAction @click="cancelEdit">Cancel</RowAction>
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
          <RowAction
            icon-only
            title="Move up"
            :disabled="index === 0"
            @click="move(template, -1)"
          >
            <Icon name="chevron-up" :size="14" />
          </RowAction>
          <RowAction
            icon-only
            title="Move down"
            :disabled="index === templates.length - 1"
            @click="move(template, 1)"
          >
            <Icon name="chevron-down" :size="14" />
          </RowAction>
          <RowAction icon-only title="Edit name & color" @click="startEdit(template)">
            <Icon name="pencil" :size="13" />
          </RowAction>
          <RowAction
            icon-only
            tone="danger"
            title="Delete status"
            @click="deleteTargetId = template.id"
          >
            <Icon name="trash" :size="13" />
          </RowAction>
        </template>
      </div>
    </div>

    <div v-if="templates.length === 0 && !adding" class="atl-statuses-empty">
      {{ emptyLabel }}
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
      <RowAction @click="adding = false">Cancel</RowAction>
    </div>
    <Btn v-else variant="secondary" style="margin-top: 12px;" @click="adding = true">
      <Icon name="plus" :size="14" />Add status
    </Btn>

    <ConfirmDialog
      :open="deleteTargetId !== null"
      tone="danger"
      :title="deleteTitle"
      :message="deleteMessage"
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
</style>
