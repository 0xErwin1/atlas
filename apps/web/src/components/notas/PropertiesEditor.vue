<script setup lang="ts">
import { ref, watch } from 'vue';
import Icon from '@/components/ui/Icon.vue';

/**
 * Inline, editable frontmatter properties block, shown below the note title
 * (Obsidian "Properties" style). The markdown frontmatter is the source of
 * truth: rows are seeded from `meta` and emitted back as a rebuilt meta object on
 * every edit, which the Notes view persists through the normal save path.
 *
 * `meta` is re-read into the rows only when it changes from the OUTSIDE (a new
 * document loads), detected by comparing against the last value we emitted. An
 * echo of our own edit is ignored so typing never resets the inputs (mirrors the
 * lastEmitted pattern in MarkdownEditor).
 */

const props = defineProps<{
  meta: Record<string, unknown>;
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
}

function removeRow(index: number): void {
  rows.value.splice(index, 1);
  emitChange();
}
</script>

<template>
  <div class="properties">
    <div v-if="rows.length > 0" class="properties-block">
      <div v-for="(row, index) in rows" :key="index" class="property-row">
        <input
          v-model="row.key"
          type="text"
          class="property-key"
          placeholder="property"
          spellcheck="false"
          @input="emitChange"
        />
        <input
          v-model="row.value"
          type="text"
          class="property-value"
          placeholder="empty"
          spellcheck="false"
          @input="emitChange"
        />
        <button
          type="button"
          class="property-remove"
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
  margin-bottom: 18px;
}

.properties-block {
  border-top: 1px solid var(--c-border);
  border-bottom: 1px solid var(--c-border);
  padding: 6px 0;
  display: flex;
  flex-direction: column;
  gap: 1px;
}

.property-row {
  display: flex;
  align-items: center;
  gap: 8px;
}

.property-key,
.property-value {
  height: 26px;
  padding: 0 6px;
  border: none;
  background: transparent;
  border-radius: var(--r-sm);
  font-family: var(--font-mono);
  font-size: var(--fs-sm);
  color: var(--c-foreground);
  outline: none;
}

.property-key {
  width: 140px;
  flex-shrink: 0;
  color: var(--c-muted);
}

.property-value {
  flex: 1;
  min-width: 0;
}

.property-key:hover,
.property-value:hover,
.property-key:focus,
.property-value:focus {
  background: var(--c-input);
}

.property-remove {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  flex-shrink: 0;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--c-muted);
  border-radius: var(--r-sm);
  cursor: pointer;
  opacity: 0;
}

.property-row:hover .property-remove {
  opacity: 1;
}

.property-remove:hover {
  background: var(--c-raised);
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
  font-family: var(--font-mono);
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
