<script setup lang="ts">
import * as lucide from 'lucide-vue-next';
import { computed } from 'vue';

const props = withDefaults(
  defineProps<{
    name: string;
    size?: number;
  }>(),
  {
    size: 16,
  },
);

const ATLAS_GLYPH_SVG = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" fill="currentColor" aria-hidden="true" style="width:100%;height:100%"><path d="M16 2 L30 27 L2 27 Z" fill="none" stroke="currentColor" stroke-width="2"/><circle cx="16" cy="16" r="4" fill="currentColor"/><line x1="16" y1="12" x2="16" y2="5" stroke="currentColor" stroke-width="1.5"/></svg>`;

const isGlyph = computed(() => props.name === 'atlas-glyph');

const lucideIcon = computed(() => {
  if (isGlyph.value) return null;
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
      v-if="isGlyph"
      :style="{ width: `${size}px`, height: `${size}px`, display: 'inline-flex', color: 'inherit' }"
      v-html="ATLAS_GLYPH_SVG"
    />
    <component
      :is="lucideIcon"
      v-else-if="lucideIcon"
      :size="size"
    />
  </span>
</template>
