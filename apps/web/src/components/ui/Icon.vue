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
  // Brand mark — diverging legs + crossbar + filled apex, matching the
  // hi-fi `atlas` glyph (frames/icons.jsx) and the app-icon artwork.
  'atlas-glyph': {
    paths:
      '<path d="M12 5 5.5 20M12 5 18.5 20M8.3 13.6H15.7"/><circle cx="5.5" cy="20" r="1.6"/><circle cx="18.5" cy="20" r="1.6"/><circle cx="12" cy="4" r="1.8" fill="currentColor" stroke="none"/>',
  },
  // Rail identity glyphs ported verbatim from the hi-fi icon set so the rail
  // matches the design exactly (lucide's file-text/kanban differ visibly).
  notes: {
    paths:
      '<path d="M14 3v5h5"/><path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M9 12h6M9 16h5"/>',
  },
  tasks: {
    paths: '<path d="M3 5l2 2 3-3"/><path d="M3 13l2 2 3-3"/><path d="M11 5h10M11 14h10M11 19h6"/>',
  },
  dashboard: {
    paths:
      '<rect x="3" y="3" width="8" height="10" rx="1"/><rect x="13" y="3" width="8" height="6" rx="1"/><rect x="13" y="13" width="8" height="8" rx="1"/><rect x="3" y="17" width="8" height="4" rx="1"/>',
  },
  // Reading-width toggle in the editor mode control (hi-fi `widen`).
  widen: {
    paths: '<path d="M12 3v18M3 12h18M8 7 3 12l5 5M16 7l5 5-5 5"/>',
  },
  dot: {
    fill: true,
    paths: '<circle cx="12" cy="12" r="3.5"/>',
  },
  enter: {
    paths: '<path d="M9 10l-4 4 4 4"/><path d="M5 14h11a4 4 0 0 0 4-4V6"/>',
  },
  command: {
    paths: '<path d="M9 6a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3z"/>',
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
