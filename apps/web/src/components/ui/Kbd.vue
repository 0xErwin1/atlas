<script setup lang="ts">
defineProps<{
  label?: string;
}>();
</script>

<template>
  <kbd
    class="inline-flex items-center justify-center shrink-0 select-none"
    style="
      padding: 1px 5px;
      min-width: 20px;
      border-radius: var(--r-sm);
      background-color: var(--c-raised);
      border: 1px solid var(--c-border);
      box-shadow: 0 1px 0 var(--c-border);
      color: var(--c-muted);
      font-family: var(--font-mono);
      font-size: var(--fs-xs);
      font-weight: var(--fw-medium);
      line-height: 1.6;
    "
  >
    <slot>{{ label }}</slot>
  </kbd>
</template>
