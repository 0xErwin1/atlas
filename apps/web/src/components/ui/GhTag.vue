<script setup lang="ts">
withDefaults(
  defineProps<{
    color?: string;
    label?: string;
  }>(),
  {
    color: 'var(--c-muted)',
    label: '',
  },
);
</script>

<template>
  <span
    class="inline-flex items-center gap-1 shrink-0 select-none"
    style="
      padding: 1px 7px;
      border-radius: var(--r-full);
      font-family: var(--font-mono);
      font-size: var(--fs-xs);
      font-weight: var(--fw-medium);
      line-height: 1.5;
      border: 1px solid var(--c-border);
    "
    :style="{
      backgroundColor: `${color}22`,
      color: color,
    }"
  >
    <slot>{{ label }}</slot>
  </span>
</template>
