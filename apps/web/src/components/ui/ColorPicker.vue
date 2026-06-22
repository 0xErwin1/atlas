<script setup lang="ts">
import { ref, watch } from 'vue';
import { SWATCHES } from '@/lib/swatches';

/**
 * Small color picker for a label / status / tag. Presentational: the parent owns
 * open state and anchoring, and persists the choice. Offers the seven named
 * swatches plus a custom `#RRGGBB` hex (native color input mirrored by a text
 * field). Emits the selected value — a swatch id or a hex string — through the
 * same `select` event, so callers treat both uniformly.
 */

const HEX_COLOR = /^#[0-9A-Fa-f]{6}$/;

const props = defineProps<{
  /** Currently applied swatch id or hex, highlighted (swatch) or shown (hex). */
  selected?: string;
}>();

const emit = defineEmits<{
  select: [value: string];
}>();

function isHex(value: string | undefined): value is string {
  return value !== undefined && HEX_COLOR.test(value);
}

/** The text field shows a custom hex selection; named swatches leave it empty. */
const hexDraft = ref(isHex(props.selected) ? props.selected : '');

watch(
  () => props.selected,
  (next) => {
    hexDraft.value = isHex(next) ? next : '';
  },
);

function onHexInput(event: Event): void {
  const value = (event.target as HTMLInputElement).value;
  hexDraft.value = value;
  if (isHex(value)) emit('select', value);
}

function onNativeInput(event: Event): void {
  const value = (event.target as HTMLInputElement).value;
  hexDraft.value = value;
  if (isHex(value)) emit('select', value);
}
</script>

<template>
  <div class="color-picker" role="menu" aria-label="Pick a color">
    <div class="swatch-row">
      <button
        v-for="swatch in SWATCHES"
        :key="swatch.id"
        type="button"
        class="swatch"
        :class="{ on: swatch.id === selected }"
        :title="swatch.label"
        :aria-label="swatch.label"
        :style="{ background: swatch.bg, borderColor: swatch.border }"
        @click="emit('select', swatch.id)"
      >
        <span class="dot" :style="{ background: swatch.fg }" />
      </button>
    </div>

    <div class="hex-row">
      <label class="native-wrap" title="Pick a custom color">
        <span class="native-dot" :style="{ background: isHex(hexDraft) ? hexDraft : 'transparent' }" />
        <input
          type="color"
          class="native"
          :value="isHex(hexDraft) ? hexDraft : '#000000'"
          aria-label="Custom color"
          @input="onNativeInput"
        />
      </label>
      <input
        type="text"
        class="hex-text"
        placeholder="#RRGGBB"
        maxlength="7"
        spellcheck="false"
        autocapitalize="off"
        aria-label="Hex color"
        :value="hexDraft"
        @input="onHexInput"
      />
    </div>
  </div>
</template>

<style scoped>
.color-picker {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 8px;
  width: max-content;
}

.swatch-row {
  display: flex;
  gap: 5px;
}

.swatch {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  padding: 0;
  border: 1px solid transparent;
  border-radius: var(--r-sm);
  cursor: pointer;
}

.swatch.on {
  box-shadow: 0 0 0 2px var(--c-primary);
}

.dot {
  width: 8px;
  height: 8px;
  border-radius: var(--r-full);
}

.hex-row {
  display: flex;
  align-items: center;
  gap: 6px;
  padding-top: 6px;
  border-top: 1px solid var(--c-border);
}

.native-wrap {
  position: relative;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  border: 1px solid var(--c-border);
  border-radius: var(--r-sm);
  cursor: pointer;
  overflow: hidden;
}

.native-dot {
  width: 12px;
  height: 12px;
  border-radius: var(--r-full);
  border: 1px solid var(--c-border);
}

.native {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  padding: 0;
  border: none;
  opacity: 0;
  cursor: pointer;
}

.hex-text {
  width: 92px;
  height: 24px;
  padding: 0 8px;
  background: var(--c-raised);
  border: 1px solid var(--c-border);
  border-radius: var(--r-sm);
  font-family: var(--font-mono);
  font-size: 12px;
  color: var(--c-foreground);
  outline: none;
}

.hex-text:focus {
  border-color: var(--c-primary);
}
</style>
