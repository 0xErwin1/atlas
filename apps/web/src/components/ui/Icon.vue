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

// Glyphs with no faithful lucide equivalent live as SVG files under
// `src/assets/icons` (ported from the hi-fi icon set). They use currentColor on a
// 24-viewBox so they inherit color like lucide icons; stroke width is driven by
// the `strokeWidth` prop through CSS (see the scoped style below).
const CUSTOM_SVGS = import.meta.glob('../../assets/icons/*.svg', {
  query: '?raw',
  import: 'default',
  eager: true,
}) as Record<string, string>;

const CUSTOM_BY_NAME: Record<string, string> = {};
for (const [filePath, svg] of Object.entries(CUSTOM_SVGS)) {
  const fileName = filePath.split('/').pop() ?? '';
  CUSTOM_BY_NAME[fileName.replace('.svg', '')] = svg;
}

const customSvg = computed(() => CUSTOM_BY_NAME[props.name] ?? '');
const isCustom = computed(() => props.name in CUSTOM_BY_NAME);

const lucideIcon = computed(() => {
  if (isCustom.value) return null;
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
      v-if="isCustom"
      class="atl-icon-custom"
      :style="{ width: `${size}px`, height: `${size}px` }"
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

<style scoped>
.atl-icon-custom {
  display: inline-flex;
  color: inherit;
}

.atl-icon-custom :deep(svg) {
  width: 100%;
  height: 100%;
  stroke-width: v-bind(strokeWidth);
}
</style>
