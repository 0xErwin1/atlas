<script setup lang="ts">
withDefaults(
  defineProps<{
    label?: string;
  }>(),
  {
    label: 'AGENT',
  },
);
</script>

<template>
  <span
    class="inline-flex items-center shrink-0 select-none"
    style="
      padding: 0 4px;
      background-color: var(--c-agent-bg);
      border: 1px solid var(--c-agent-border);
      color: var(--c-agent);
      font-family: var(--font-mono);
      font-size: var(--fs-label);
      font-weight: var(--fw-bold);
      letter-spacing: var(--ls-label);
      line-height: 14px;
    "
  >
    <slot>{{ label }}</slot>
  </span>
</template>
