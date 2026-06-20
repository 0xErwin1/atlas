<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue';
import Avatar from '@/components/ui/Avatar.vue';
import Chip from '@/components/ui/Chip.vue';
import ColorPicker from '@/components/ui/ColorPicker.vue';
import Icon from '@/components/ui/Icon.vue';
import Popover from '@/components/ui/Popover.vue';
import TagInput from '@/components/ui/TagInput.vue';
import { useLabelColorsStore } from '@/stores/labelColors';
import { useTagsStore } from '@/stores/tags';

const labelColors = useLabelColorsStore();
const tagsStore = useTagsStore();

/**
 * Inline, editable frontmatter properties block, shown below the note title
 * (Obsidian "Properties" style). The markdown frontmatter is the source of
 * truth: rows are seeded from `meta` and emitted back as a rebuilt meta object on
 * every edit, which the Notes view persists through the normal save path.
 *
 * Known keys render as typed widgets (status → toned chip, tags → chips with
 * add/remove, visibility → lock/globe, owner → avatar) while staying editable;
 * unknown keys fall back to a plain value cell. `meta` is re-read into the rows
 * only when it changes from the OUTSIDE (a new document loads), detected by
 * comparing against the last value we emitted, so typing never resets the inputs.
 */

const props = defineProps<{
  meta: Record<string, unknown>;
  /** Workspace slug, used to load/create tags in the shared registry. */
  ws: string;
}>();

const emit = defineEmits<{
  change: [meta: Record<string, unknown>];
}>();

interface Row {
  key: string;
  value: string;
  /** Preserve list-valued frontmatter (e.g. tags) as a YAML sequence on save. */
  isArray: boolean;
}

function rowsFrom(meta: Record<string, unknown>): Row[] {
  return Object.entries(meta).map(([key, value]) => ({
    key,
    value: Array.isArray(value)
      ? value.map((v) => String(v)).join(', ')
      : value === null || value === undefined
        ? ''
        : String(value),
    isArray: Array.isArray(value),
  }));
}

const rows = ref<Row[]>(rowsFrom(props.meta));
let lastEmitted = JSON.stringify(props.meta);

function buildMeta(): Record<string, unknown> {
  const meta: Record<string, unknown> = {};

  for (const row of rows.value) {
    const key = row.key.trim();
    if (key === '') continue;

    meta[key] = row.isArray
      ? row.value
          .split(',')
          .map((part) => part.trim())
          .filter((part) => part !== '')
      : row.value;
  }

  return meta;
}

function emitChange(): void {
  const meta = buildMeta();
  lastEmitted = JSON.stringify(meta);
  emit('change', meta);
}

watch(
  () => props.meta,
  (meta) => {
    const serialized = JSON.stringify(meta);
    if (serialized === lastEmitted) return;
    rows.value = rowsFrom(meta);
    lastEmitted = serialized;
  },
);

function addRow(): void {
  rows.value.push({ key: '', value: '', isArray: false });
  editing.value = rows.value.length - 1;
}

function removeRow(index: number): void {
  rows.value.splice(index, 1);
  if (editing.value === index) editing.value = null;
  emitChange();
}

// ── typed value rendering ────────────────────────────────────────────
type Kind = 'tags' | 'status' | 'visibility' | 'owner' | 'text';

function rowKind(row: Row): Kind {
  if (row.isArray) return 'tags';

  const key = row.key.trim().toLowerCase();
  if (key === 'status') return 'status';
  if (key === 'visibility' || key === 'access') return 'visibility';
  if (key === 'owner' || key === 'author' || key === 'assignee') return 'owner';
  return 'text';
}

// Color is a user choice, persisted per value — never inferred from the text.
function statusKey(value: string): string {
  return `status:${value.trim().toLowerCase()}`;
}

function tagKey(tag: string): string {
  return `tag:${tag.trim().toLowerCase()}`;
}

function pickColor(key: string, swatchId: string): void {
  labelColors.setColor(key, swatchId);
}

function visibilityIcon(value: string): string {
  return value.toLowerCase().includes('public') ? 'globe' : 'lock';
}

function ownerInitials(value: string): string {
  const clean = value.replace(/^@/, '').trim();
  return (clean.slice(0, 2) || '?').toUpperCase();
}

function keyLabel(key: string): string {
  const trimmed = key.trim();
  return trimmed === '' ? '' : trimmed.charAt(0).toUpperCase() + trimmed.slice(1);
}

// ── tags: creatable combobox over the shared registry ────────────────
function tagList(row: Row): string[] {
  return row.value
    .split(',')
    .map((t) => t.trim())
    .filter((t) => t !== '');
}

function setRowTags(row: Row, tags: string[]): void {
  row.value = tags.join(', ');
  emitChange();
}

// Autocomplete pool: the workspace tag registry unioned with tags already seen
// in loaded data, deduped case-insensitively.
const tagSuggestions = computed<string[]>(() => {
  const byLower = new Map<string, string>();
  for (const name of [...tagsStore.names, ...labelColors.tagNames]) {
    const key = name.trim().toLowerCase();
    if (key !== '' && !byLower.has(key)) byLower.set(key, name.trim());
  }
  return [...byLower.values()].sort((a, b) => a.localeCompare(b));
});

function onCreateTag(name: string): void {
  void tagsStore.ensure(props.ws, name);
}

onMounted(() => {
  void tagsStore.load(props.ws);
});

// ── click-to-edit for single-value typed cells ───────────────────────
// `editing` holds the index of the row whose value is currently an input; the
// typed display is shown otherwise. The function ref focuses the input the moment
// it mounts so a click lands the caret without an extra tab.
const editing = ref<number | null>(null);

function startEdit(index: number): void {
  editing.value = index;
}

function focusOnMount(el: unknown): void {
  if (el instanceof HTMLElement) nextTick(() => el.focus());
}
</script>

<template>
  <div class="properties">
    <div v-if="rows.length > 0" class="properties-card">
      <div v-for="(row, index) in rows" :key="index" class="meta-row">
        <input
          v-model="row.key"
          type="text"
          class="meta-key"
          placeholder="property"
          spellcheck="false"
          @input="emitChange"
        />

        <!-- tags: creatable combobox; click a chip to recolor, × to remove -->
        <div v-if="rowKind(row) === 'tags'" class="meta-value tags">
          <TagInput
            :model-value="tagList(row)"
            :suggestions="tagSuggestions"
            :color-for="(t) => labelColors.colorFor(tagKey(t))"
            :on-recolor="(t, id) => pickColor(tagKey(t), id)"
            placeholder="add…"
            @update:model-value="(next) => setRowTags(row, next)"
            @create="onCreateTag"
          />
        </div>

        <!-- single-value typed cells: typed display ⇄ click-to-edit input -->
        <template v-else>
          <input
            v-if="editing === index"
            :ref="focusOnMount"
            v-model="row.value"
            type="text"
            class="meta-value-input"
            placeholder="empty"
            spellcheck="false"
            @input="emitChange"
            @blur="editing = null"
            @keydown.enter.prevent="editing = null"
          />

          <!-- status: colored chip; click recolors, pencil edits the value -->
          <div
            v-else-if="rowKind(row) === 'status' && row.value !== ''"
            class="meta-value status-cell"
          >
            <Popover placement="bottom-start">
              <template #trigger="{ toggle }">
                <Chip
                  :color="labelColors.colorFor(statusKey(row.value))"
                  icon="dot"
                  style="cursor: pointer;"
                  :title="`Recolor “${row.value}”`"
                  @click="toggle"
                >
                  {{ row.value }}
                </Chip>
              </template>
              <template #default="{ close }">
                <ColorPicker
                  :selected="labelColors.colorFor(statusKey(row.value))"
                  @select="(id) => (pickColor(statusKey(row.value), id), close())"
                />
              </template>
            </Popover>
            <button
              type="button"
              class="status-edit"
              title="Edit value"
              aria-label="Edit value"
              @click="startEdit(index)"
            >
              <Icon name="pencil" :size="11" />
            </button>
          </div>

          <button
            v-else
            type="button"
            class="meta-value display"
            @click="startEdit(index)"
          >
            <template v-if="row.value === ''">
              <span class="empty">Empty</span>
            </template>

            <template v-else-if="rowKind(row) === 'visibility'">
              <Icon :name="visibilityIcon(row.value)" :size="12" style="color: var(--c-muted);" />
              <span>{{ row.value }}</span>
            </template>

            <template v-else-if="rowKind(row) === 'owner'">
              <Avatar :name="ownerInitials(row.value)" :size="18" />
              <span class="mono">{{ row.value }}</span>
            </template>

            <span v-else>{{ row.value }}</span>
          </button>
        </template>

        <button
          type="button"
          class="meta-remove"
          title="Remove property"
          aria-label="Remove property"
          @click="removeRow(index)"
        >
          <Icon name="x" :size="13" />
        </button>
      </div>
    </div>

    <button type="button" class="property-add" @click="addRow">
      <Icon name="plus" :size="13" />
      Add property
    </button>
  </div>
</template>

<style scoped>
.properties {
  margin-bottom: 22px;
}

.properties-card {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 10px 14px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-lg);
  background: var(--c-raised);
}

.meta-row {
  display: flex;
  align-items: center;
  gap: 10px;
  min-height: 22px;
  font-size: var(--fs-sm);
}

.meta-key {
  width: 88px;
  flex: 0 0 auto;
  height: 22px;
  padding: 0 4px;
  border: none;
  border-radius: var(--r-sm);
  background: transparent;
  color: var(--c-muted);
  font-family: var(--font-ui);
  font-size: var(--fs-sm);
  outline: none;
}

.meta-key:hover,
.meta-key:focus {
  background: var(--c-input);
}

.meta-value {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
}

/* Click-to-edit display cell: a borderless button that reads like text/chips. */
.meta-value.display {
  border: none;
  background: transparent;
  padding: 2px 4px;
  margin: 0 -4px;
  border-radius: var(--r-sm);
  cursor: text;
  text-align: left;
  color: var(--c-foreground);
}

.meta-value.display:hover {
  background: var(--c-input);
}

.meta-value.display .empty {
  color: var(--c-muted);
}

.meta-value.display .mono {
  font-family: var(--font-mono);
}

.meta-value-input {
  flex: 1;
  min-width: 0;
  height: 22px;
  padding: 0 4px;
  border: none;
  border-radius: var(--r-sm);
  background: var(--c-input);
  color: var(--c-foreground);
  font-family: var(--font-ui);
  font-size: var(--fs-sm);
  outline: none;
}

.status-cell {
  position: relative;
}

.status-edit {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  padding: 0;
  border: none;
  border-radius: var(--r-sm);
  background: transparent;
  color: var(--c-muted);
  cursor: pointer;
  opacity: 0;
}

.status-cell:hover .status-edit {
  opacity: 1;
}

.status-edit:hover {
  background: var(--c-input);
  color: var(--c-foreground);
}

.meta-remove {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  flex: 0 0 auto;
  padding: 0;
  border: none;
  border-radius: var(--r-sm);
  background: transparent;
  color: var(--c-muted);
  cursor: pointer;
  opacity: 0;
}

.meta-row:hover .meta-remove {
  opacity: 1;
}

.meta-remove:hover {
  background: var(--c-background);
  color: var(--c-danger);
}

.property-add {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  margin-top: 6px;
  padding: 3px 6px;
  border: none;
  background: transparent;
  color: var(--c-muted);
  border-radius: var(--r-sm);
  font-family: var(--font-ui);
  font-size: var(--fs-sm);
  cursor: pointer;
  opacity: 0;
  transition: opacity 0.1s ease;
}

.properties:hover .property-add,
.property-add:focus {
  opacity: 1;
}

.property-add:hover {
  background: var(--c-raised);
  color: var(--c-foreground);
}
</style>
