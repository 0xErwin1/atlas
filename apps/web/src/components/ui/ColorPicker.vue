<script setup lang="ts">
import { SWATCHES } from '@/lib/swatches';

/**
 * Small swatch popover for choosing a label / status / tag color. Presentational:
 * the parent owns open state and anchoring, and persists the choice (labelColors
 * store). Emits the selected swatch id.
 */

defineProps<{
  /** Currently applied swatch id, highlighted with a ring. */
  selected?: string;
}>();

const emit = defineEmits<{
  select: [swatchId: string];
}>();
</script>

<template>
  <div class="atl-menu color-picker" role="menu" aria-label="Pick a color">
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
</template>

<style scoped>
.color-picker {
  display: flex;
  gap: 5px;
  padding: 6px;
  width: max-content;
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
</style>
