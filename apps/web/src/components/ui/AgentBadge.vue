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
      border-radius: 2px;
      background-color: var(--c-agent-bg);
      border: 1px solid var(--c-agent-border);
      color: var(--c-agent);
      font-family: var(--font-mono);
      font-size: 9px;
      font-weight: var(--fw-bold);
      letter-spacing: 0.06em;
      line-height: 14px;
    "
  >
    <slot>{{ label }}</slot>
  </span>
</template>
