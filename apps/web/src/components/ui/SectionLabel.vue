<script setup lang="ts">
defineProps<{
  /** When true, removes the left padding (used inside padded inspector panels). */
  flush?: boolean;
}>();
</script>

<template>
  <div
    class="select-none"
    :style="`
      font-family: var(--font-mono);
      font-size: var(--fs-label);
      font-weight: var(--fw-semibold);
      letter-spacing: var(--ls-label);
      text-transform: uppercase;
      color: var(--c-muted);
      padding: ${flush ? '10px 0 4px' : '10px 10px 4px'};
    `"
  >
    <slot />
  </div>
</template>
