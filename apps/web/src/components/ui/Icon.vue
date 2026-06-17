<script setup lang="ts">
import * as lucide from 'lucide-vue-next';
import { computed } from 'vue';

const props = withDefaults(
  defineProps<{
    name: string;
    size?: number;
    strokeWidth?: number;
  }>(),
  {
    size: 16,
    strokeWidth: 1.8,
  },
);

// Glyphs with no faithful lucide equivalent are ported verbatim from the
// hi-fi design icon set (frames/icons.jsx). Paths use currentColor on a
// 24-viewBox so they inherit size/color exactly like lucide icons.
const CUSTOM_GLYPHS: Record<string, { fill?: boolean; paths: string }> = {
  'atlas-glyph': {
    paths:
      '<circle cx="12" cy="12" r="9"/><path d="M12 3c3 3 3 15 0 18M12 3c-3 3-3 15 0 18M4 9h16M4 15h16"/>',
  },
  dot: {
    fill: true,
    paths: '<circle cx="12" cy="12" r="3.5"/>',
  },
};

const customGlyph = computed(() => CUSTOM_GLYPHS[props.name] ?? null);

const customSvg = computed(() => {
  const glyph = customGlyph.value;
  if (glyph === null) return '';
  const fill = glyph.fill === true ? 'currentColor' : 'none';
  const stroke = glyph.fill === true ? 'none' : 'currentColor';
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="${fill}" stroke="${stroke}" stroke-width="${props.strokeWidth}" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" style="width:100%;height:100%">${glyph.paths}</svg>`;
});

const lucideIcon = computed(() => {
  if (customGlyph.value !== null) return null;
  const key = props.name
    .split('-')
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join('');
  return (lucide as Record<string, unknown>)[key] ?? null;
});
</script>

<template>
  <span
    class="inline-flex items-center justify-center shrink-0"
    :style="{ width: `${size}px`, height: `${size}px` }"
    aria-hidden="true"
  >
    <span
      v-if="customGlyph"
      :style="{ width: `${size}px`, height: `${size}px`, display: 'inline-flex', color: 'inherit' }"
      v-html="customSvg"
    />
    <component
      :is="lucideIcon"
      v-else-if="lucideIcon"
      :size="size"
      :stroke-width="strokeWidth"
    />
  </span>
</template>
