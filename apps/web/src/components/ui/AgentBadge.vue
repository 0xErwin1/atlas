<script setup lang="ts">
withDefaults(
  defineProps<{
    label?: string;
  }>(),
  {
    label: 'AI',
  },
);
</script>

<template>
  <span
    class="inline-flex items-center gap-1 shrink-0 select-none"
    style="
      padding: 1px 6px;
      border-radius: var(--r-full);
      background-color: var(--c-agent-bg);
      border: 1px solid var(--c-agent-border);
      color: var(--c-agent);
      font-family: var(--font-mono);
      font-size: var(--fs-xs);
      font-weight: var(--fw-semibold);
      line-height: 1.4;
    "
  >
    <span style="font-size: 9px">✦</span>
    <slot>{{ label }}</slot>
  </span>
</template>
