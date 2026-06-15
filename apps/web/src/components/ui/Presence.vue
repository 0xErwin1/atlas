<script setup lang="ts">
withDefaults(
  defineProps<{
    active?: boolean;
    size?: number;
  }>(),
  {
    active: true,
    size: 8,
  },
);
</script>

<template>
  <span
    :style="{
      display: 'inline-block',
      width: `${size}px`,
      height: `${size}px`,
      borderRadius: '50%',
      backgroundColor: active ? 'var(--c-success)' : 'var(--c-muted)',
      flexShrink: 0,
    }"
    :class="active ? 'animate-pulse' : ''"
    aria-hidden="true"
  />
</template>
