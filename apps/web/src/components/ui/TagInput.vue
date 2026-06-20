<script setup lang="ts">
import { computed, nextTick, ref } from 'vue';
import Chip from '@/components/ui/Chip.vue';
import ColorPicker from '@/components/ui/ColorPicker.vue';
import Icon from '@/components/ui/Icon.vue';
import Popover from '@/components/ui/Popover.vue';
import { swatchById } from '@/lib/swatches';

/**
 * Creatable tag combobox: selected tags render as removable chips followed by a
 * text input. Typing filters `suggestions` (case-insensitive, excluding already
 * selected ones); picking a suggestion selects it, while a value matching nothing
 * offers a "Create" row. Selection is the `string[]` v-model; a brand-new value
 * (absent from `suggestions`) also emits `create` so the host can register it.
 *
 * It is intentionally storage-agnostic — the host supplies the suggestion pool
 * and an optional `colorFor` for chip swatches, and persists the model however it
 * needs (task labels, frontmatter, …). When `onRecolor` is also supplied, each
 * chip opens a color picker so the host can persist the chosen swatch.
 */

const model = defineModel<string[]>({ required: true });

const props = withDefaults(
  defineProps<{
    suggestions?: string[];
    placeholder?: string;
    /** Maps a tag name to a swatch id for chip coloring. */
    colorFor?: (tag: string) => string;
    /** When set, chips become recolorable; receives the picked swatch id. */
    onRecolor?: (tag: string, swatchId: string) => void;
  }>(),
  {
    suggestions: () => [],
    placeholder: '+ Tag',
    colorFor: undefined,
    onRecolor: undefined,
  },
);

const emit = defineEmits<{
  /** A value not present in `suggestions` was added. */
  create: [name: string];
}>();

const draft = ref('');
const focused = ref(false);
const activeIndex = ref(-1);
const inputRef = ref<HTMLInputElement | null>(null);

function lower(value: string): string {
  return value.trim().toLowerCase();
}

const selectedLower = computed(() => new Set(model.value.map(lower)));

const available = computed(() => props.suggestions.filter((s) => !selectedLower.value.has(lower(s))));

const filtered = computed(() => {
  const q = lower(draft.value);
  const pool = q === '' ? available.value : available.value.filter((s) => s.toLowerCase().includes(q));
  return pool.slice(0, 50);
});

const trimmed = computed(() => draft.value.trim());

const isKnown = computed(() => {
  const q = lower(trimmed.value);
  if (q === '') return true;
  return selectedLower.value.has(q) || props.suggestions.some((s) => lower(s) === q);
});

const canCreate = computed(() => trimmed.value !== '' && !isKnown.value);

const showPanel = computed(() => focused.value && (filtered.value.length > 0 || canCreate.value));

function swatchFor(tag: string): string {
  return props.colorFor?.(tag) ?? '';
}

function dotColor(tag: string): string {
  const id = swatchFor(tag);
  return id === '' ? 'var(--c-muted)' : swatchById(id).fg;
}

function add(name: string): void {
  const value = name.trim();
  draft.value = '';
  activeIndex.value = -1;
  if (value === '') return;

  if (selectedLower.value.has(value.toLowerCase())) return;

  const known = props.suggestions.some((s) => lower(s) === value.toLowerCase());
  if (!known) emit('create', value);

  model.value = [...model.value, value];
}

function remove(tag: string): void {
  model.value = model.value.filter((t) => t !== tag);
}

function onEnter(): void {
  const active = filtered.value[activeIndex.value];
  if (active !== undefined) add(active);
  else add(draft.value);
}

function onBackspace(): void {
  if (draft.value !== '') return;
  const last = model.value.at(-1);
  if (last !== undefined) remove(last);
}

function moveActive(delta: number): void {
  const max = filtered.value.length - 1;
  if (max < 0) {
    activeIndex.value = -1;
    return;
  }
  const next = activeIndex.value + delta;
  activeIndex.value = next < 0 ? max : next > max ? 0 : next;
}

function onBlur(): void {
  focused.value = false;
  activeIndex.value = -1;
}

function focusInput(): void {
  void nextTick(() => inputRef.value?.focus());
}
</script>

<template>
  <div class="atl-taginput" :class="{ focused }" @click="focusInput">
    <template v-for="tag in model" :key="tag">
      <Popover v-if="onRecolor" placement="bottom-start">
        <template #trigger="{ toggle }">
          <Chip
            :color="swatchFor(tag)"
            :tone="swatchFor(tag) === '' ? 'info' : 'neutral'"
            :title="`Recolor “${tag}”`"
            style="cursor: pointer;"
            @click.stop="toggle"
          >
            {{ tag }}
            <button
              type="button"
              class="atl-taginput-x"
              aria-label="Remove tag"
              @click.stop="remove(tag)"
            >
              <Icon name="x" :size="10" />
            </button>
          </Chip>
        </template>
        <template #default="{ close }">
          <ColorPicker
            :selected="swatchFor(tag)"
            @select="(id) => (onRecolor?.(tag, id), close())"
          />
        </template>
      </Popover>

      <Chip
        v-else
        :color="swatchFor(tag)"
        :tone="swatchFor(tag) === '' ? 'info' : 'neutral'"
      >
        {{ tag }}
        <button
          type="button"
          class="atl-taginput-x"
          aria-label="Remove tag"
          @click.stop="remove(tag)"
        >
          <Icon name="x" :size="10" />
        </button>
      </Chip>
    </template>

    <div class="atl-taginput-field">
      <input
        ref="inputRef"
        v-model="draft"
        class="atl-taginput-input"
        :placeholder="model.length === 0 ? placeholder : ''"
        spellcheck="false"
        autocomplete="off"
        @focus="focused = true"
        @blur="onBlur"
        @keydown.enter.prevent="onEnter"
        @keydown.down.prevent="moveActive(1)"
        @keydown.up.prevent="moveActive(-1)"
        @keydown.backspace="onBackspace"
        @keydown.esc.prevent="onBlur"
      />

      <div v-if="showPanel" class="atl-taginput-panel" role="listbox">
        <button
          v-for="(opt, i) in filtered"
          :key="opt"
          type="button"
          role="option"
          :aria-selected="i === activeIndex"
          class="atl-taginput-option"
          :class="{ active: i === activeIndex }"
          @mousedown.prevent
          @click="add(opt)"
        >
          <span class="atl-taginput-dot" :style="{ background: dotColor(opt) }" />
          {{ opt }}
        </button>

        <button
          v-if="canCreate"
          type="button"
          class="atl-taginput-option create"
          @mousedown.prevent
          @click="add(trimmed)"
        >
          <Icon name="plus" :size="12" style="flex: 0 0 auto;" />
          Create “{{ trimmed }}”
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.atl-taginput {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 4px;
  width: 100%;
  min-height: 26px;
  padding: 3px 5px;
  background: transparent;
  border: 1px solid transparent;
  border-radius: var(--r-lg);
  cursor: text;
  transition:
    background 0.12s,
    border-color 0.12s;
}

.atl-taginput:hover {
  background: var(--c-input);
  border-color: var(--c-border);
}

.atl-taginput.focused {
  background: var(--c-input);
  border-color: var(--c-primary);
}

.atl-taginput-x {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  border: none;
  background: transparent;
  color: inherit;
  cursor: pointer;
  opacity: 0.6;
}

.atl-taginput-x:hover {
  opacity: 1;
}

.atl-taginput-field {
  position: relative;
  flex: 1 1 80px;
  min-width: 80px;
}

.atl-taginput-input {
  width: 100%;
  height: 18px;
  padding: 0 2px;
  border: none;
  background: transparent;
  color: var(--c-foreground);
  font-family: var(--font-ui);
  font-size: 11.5px;
  outline: none;
}

.atl-taginput-panel {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  z-index: 60;
  min-width: 160px;
  max-height: 220px;
  overflow-y: auto;
  padding: 3px;
  background: var(--c-raised);
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  box-shadow: var(--shadow-md);
}

.atl-taginput-option {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
  height: 26px;
  padding: 0 7px;
  border: none;
  border-radius: 3px;
  background: transparent;
  color: var(--c-foreground);
  font-family: var(--font-ui);
  font-size: var(--fs-sm);
  text-align: left;
  cursor: pointer;
}

.atl-taginput-option:hover,
.atl-taginput-option.active {
  background: var(--c-input);
}

.atl-taginput-option.create {
  color: var(--c-muted);
}

.atl-taginput-dot {
  width: 6px;
  height: 6px;
  flex: 0 0 auto;
  border-radius: var(--r-full);
}
</style>
