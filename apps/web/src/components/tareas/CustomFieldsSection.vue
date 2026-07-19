<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import type { components } from '@/api/types.d.ts';
import ConfirmDialog from '@/components/ui/ConfirmDialog.vue';
import DatePicker from '@/components/ui/DatePicker.vue';
import Dropdown, { type DropdownOption } from '@/components/ui/Dropdown.vue';
import Icon from '@/components/ui/Icon.vue';
import MultiSelect, { type MultiSelectOption } from '@/components/ui/MultiSelect.vue';
import { useBoardsStore } from '@/stores/boards';
import { type PropertyDefinitionDto, usePropertyDefinitionsStore } from '@/stores/propertyDefinitions';
import { useTasksStore } from '@/stores/tasks';
import { useUiStore } from '@/stores/ui';

type TaskDto = components['schemas']['TaskDto'];

const props = defineProps<{
  ws: string;
  task: TaskDto;
}>();

const propDefs = usePropertyDefinitionsStore();
const boards = useBoardsStore();
const tasks = useTasksStore();
const ui = useUiStore();

const open = ref(true);
const adding = ref(false);
const pendingDelete = ref<PropertyDefinitionDto | null>(null);

const draftName = ref('');
const draftKind = ref('text');
const draftOptions = ref('');

const KIND_OPTIONS: DropdownOption[] = [
  { value: 'text', label: 'Text', icon: 'type' },
  { value: 'number', label: 'Number', icon: 'hash' },
  { value: 'boolean', label: 'Checkbox', icon: 'check' },
  { value: 'date', label: 'Date', icon: 'calendar' },
  { value: 'select', label: 'Select', icon: 'chevron-down' },
  { value: 'multi_select', label: 'Multi-select', icon: 'list-checks' },
];

const draftNeedsOptions = computed(() => draftKind.value === 'select' || draftKind.value === 'multi_select');

onMounted(() => {
  void propDefs.load(props.ws);
});

const definitions = computed(() => propDefs.definitions);

// The stored property map is keyed by each definition's key.
const values = computed<Record<string, unknown>>(
  () => (props.task.properties as Record<string, unknown> | null | undefined) ?? {},
);

function optionsOf(def: PropertyDefinitionDto): string[] {
  return Array.isArray(def.options)
    ? (def.options as unknown[]).filter((o): o is string => typeof o === 'string')
    : [];
}

function selectOptions(def: PropertyDefinitionDto): DropdownOption[] {
  return [{ value: '', label: '—' }, ...optionsOf(def).map((o) => ({ value: o, label: o }))];
}

function multiOptions(def: PropertyDefinitionDto): MultiSelectOption[] {
  return optionsOf(def).map((o) => ({ value: o, label: o }));
}

function stringValue(def: PropertyDefinitionDto): string {
  const v = values.value[def.key];
  return typeof v === 'string' ? v : '';
}

function numberValue(def: PropertyDefinitionDto): number | '' {
  const v = values.value[def.key];
  return typeof v === 'number' ? v : '';
}

function boolValue(def: PropertyDefinitionDto): boolean {
  return values.value[def.key] === true;
}

function dateValue(def: PropertyDefinitionDto): string {
  const v = values.value[def.key];
  return typeof v === 'string' ? v.slice(0, 10) : '';
}

function arrayValue(def: PropertyDefinitionDto): string[] {
  const v = values.value[def.key];
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : [];
}

/**
 * Persists a single field's value. The stored map is replaced wholesale, so the
 * new value is merged into the current map (or the key dropped when cleared) and
 * the open task is patched to reflect the change immediately.
 */
async function setValue(def: PropertyDefinitionDto, value: unknown): Promise<void> {
  const next: Record<string, unknown> = { ...values.value };

  const cleared = value === null || value === '' || (Array.isArray(value) && value.length === 0);
  if (cleared) delete next[def.key];
  else next[def.key] = value;

  const ok = await boards.updateTask(props.ws, props.task.readable_id, { properties: next });
  if (ok) tasks.patchOpenTask({ properties: next });
  else if (boards.error) ui.showBanner(boards.error, 'error');
}

function onNumberInput(def: PropertyDefinitionDto, raw: string): void {
  const trimmed = raw.trim();
  if (trimmed === '') {
    void setValue(def, null);
    return;
  }

  const parsed = Number(trimmed);
  if (Number.isNaN(parsed)) return;
  void setValue(def, parsed);
}

function onDateInput(def: PropertyDefinitionDto, raw: string): void {
  void setValue(def, raw === '' ? null : `${raw}T00:00:00Z`);
}

function resetDraft(): void {
  draftName.value = '';
  draftKind.value = 'text';
  draftOptions.value = '';
  adding.value = false;
}

async function submitAdd(): Promise<void> {
  const name = draftName.value.trim();
  if (name === '') return;

  let options: string[] | undefined;
  if (draftNeedsOptions.value) {
    options = draftOptions.value
      .split(/[\n,]/)
      .map((o) => o.trim())
      .filter((o) => o !== '');

    if (options.length === 0) {
      ui.showBanner('Add at least one option for this field', 'error');
      return;
    }
  }

  const created = await propDefs.create(props.ws, {
    name,
    kind: draftKind.value,
    applies_to: 'task',
    ...(options !== undefined ? { options } : {}),
  });

  if (created === null) {
    if (propDefs.error) ui.showBanner(propDefs.error, 'error');
    return;
  }

  resetDraft();
}

async function doDelete(): Promise<void> {
  const def = pendingDelete.value;
  pendingDelete.value = null;
  if (def === null) return;

  const ok = await propDefs.remove(props.ws, def.id);
  if (!ok && propDefs.error) ui.showBanner(propDefs.error, 'error');
}
</script>

<template>
  <div class="atl-cf">
    <div class="atl-cf-head">
      <button type="button" class="atl-cf-toggle" :aria-expanded="open" @click="open = !open">
        <Icon
          name="chevron-down"
          :size="13"
          :style="{ transform: open ? 'none' : 'rotate(-90deg)', transition: 'transform 0.12s' }"
        />
        Custom fields
      </button>
      <span style="flex: 1;" />
      <button
        type="button"
        class="atl-gbtn"
        style="width: 22px; height: 22px;"
        title="Add field"
        @click="adding = !adding"
      >
        <Icon name="plus" :size="14" />
      </button>
    </div>

    <div v-if="open" class="atl-cf-body">
      <div
        v-for="def in definitions"
        :key="def.id"
        class="group atl-cf-row"
      >
        <span class="atl-cf-label" :title="def.key">{{ def.name }}</span>

        <span class="atl-cf-control">
          <input
            v-if="def.kind === 'text'"
            type="text"
            class="atl-cf-input"
            :value="stringValue(def)"
            @change="setValue(def, ($event.target as HTMLInputElement).value)"
          />
          <input
            v-else-if="def.kind === 'number'"
            type="number"
            class="atl-cf-input"
            :value="numberValue(def)"
            @change="onNumberInput(def, ($event.target as HTMLInputElement).value)"
          />
          <input
            v-else-if="def.kind === 'boolean'"
            type="checkbox"
            class="atl-cf-checkbox"
            :checked="boolValue(def)"
            @change="setValue(def, ($event.target as HTMLInputElement).checked)"
          />
          <DatePicker
            v-else-if="def.kind === 'date'"
            :model-value="dateValue(def)"
            @update:model-value="(v: string) => onDateInput(def, v)"
          />
          <Dropdown
            v-else-if="def.kind === 'select'"
            :options="selectOptions(def)"
            :model-value="stringValue(def)"
            @change="(v: string) => setValue(def, v)"
          />
          <MultiSelect
            v-else-if="def.kind === 'multi_select'"
            :options="multiOptions(def)"
            :model-value="arrayValue(def)"
            placeholder="None"
            @update:model-value="(v: string[]) => setValue(def, v)"
          />
        </span>

        <button
          type="button"
          class="atl-cf-del"
          :aria-label="`Delete field ${def.name}`"
          title="Delete field (workspace-wide)"
          @click="pendingDelete = def"
        >
          <Icon name="x" :size="13" />
        </button>
      </div>

      <div v-if="definitions.length === 0 && !adding" class="atl-cf-empty">
        No custom fields yet.
        <button type="button" class="atl-cf-link" @click="adding = true">Add one</button>
      </div>

      <div v-if="adding" class="atl-cf-add">
        <input
          v-model="draftName"
          type="text"
          class="atl-cf-input"
          placeholder="Field name"
          @keydown.enter="submitAdd"
        />
        <Dropdown :options="KIND_OPTIONS" :model-value="draftKind" @change="(v: string) => (draftKind = v)" />
        <textarea
          v-if="draftNeedsOptions"
          v-model="draftOptions"
          class="atl-cf-options"
          rows="3"
          placeholder="One option per line"
        />
        <div class="atl-cf-add-actions">
          <button type="button" class="atl-cf-btn" @click="resetDraft">Cancel</button>
          <button type="button" class="atl-cf-btn primary" :disabled="draftName.trim() === ''" @click="submitAdd">
            Add field
          </button>
        </div>
      </div>
    </div>

    <ConfirmDialog
      :open="pendingDelete !== null"
      title="Delete custom field"
      :message="`Delete '${pendingDelete?.name}' for the whole workspace? Existing values on tasks are kept but the field stops showing.`"
      confirm-label="Delete"
      tone="danger"
      @confirm="doDelete"
      @cancel="pendingDelete = null"
    />
  </div>
</template>

<style scoped>
.atl-cf-head {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 6px;
}

.atl-cf-toggle {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  padding: 0;
  border: none;
  background: transparent;
  cursor: pointer;
  font-size: var(--fs-xs);
  font-weight: var(--fw-semibold);
  letter-spacing: 0.06em;
  text-transform: uppercase;
  color: var(--c-muted);
}

.atl-cf-body {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.atl-cf-row {
  display: flex;
  align-items: center;
  gap: 10px;
  min-height: 30px;
}

.atl-cf-label {
  width: 132px;
  flex: 0 0 132px;
  min-width: 0;
  font-size: var(--fs-sm);
  color: var(--c-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.atl-cf-control {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
}

.atl-cf-input {
  height: 28px;
  width: 100%;
  max-width: 240px;
  padding: 0 10px;
  background: var(--c-raised);
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  color: var(--c-foreground);
  font-family: var(--font-mono);
  font-size: var(--fs-sm);
  outline: none;
}

.atl-cf-input:focus {
  border-color: var(--c-primary);
}

.atl-cf-checkbox {
  width: 15px;
  height: 15px;
  accent-color: var(--c-primary);
  cursor: pointer;
}

.atl-cf-del {
  flex: 0 0 auto;
  width: 16px;
  height: 16px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border: none;
  background: transparent;
  color: var(--c-muted);
  cursor: pointer;
  opacity: 0;
}

.group:hover .atl-cf-del {
  opacity: 1;
}

.atl-cf-empty {
  display: flex;
  align-items: center;
  gap: 8px;
  height: 38px;
  padding: 0 8px;
  border: 1px solid var(--c-border);
  border-radius: 4px;
  font-size: var(--fs-sm);
  color: var(--c-muted);
}

.atl-cf-link {
  border: none;
  background: transparent;
  color: var(--c-primary);
  cursor: pointer;
  font-size: var(--fs-sm);
  padding: 0;
}

.atl-cf-add {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-top: 4px;
  padding: 10px;
  border: 1px solid var(--c-border);
  border-radius: 4px;
  background: var(--c-panel);
}

.atl-cf-options {
  width: 100%;
  resize: vertical;
  padding: 8px 10px;
  background: var(--c-raised);
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  color: var(--c-foreground);
  font-family: var(--font-mono);
  font-size: var(--fs-sm);
  outline: none;
}

.atl-cf-add-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
}

.atl-cf-btn {
  height: 28px;
  padding: 0 12px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  background: var(--c-secondary);
  color: var(--c-foreground);
  font-family: var(--font-ui);
  font-size: var(--fs-sm);
  cursor: pointer;
}

.atl-cf-btn.primary {
  background: var(--c-primary);
  color: var(--c-primary-fg);
  border-color: transparent;
  font-weight: var(--fw-semibold);
}

.atl-cf-btn:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}
</style>
