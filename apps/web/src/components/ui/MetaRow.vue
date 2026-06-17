<script setup lang="ts">
defineProps<{
  label: string;
}>();
</script>

<template>
  <div
    class="flex items-center"
    style="
      gap: 10px;
      min-height: 22px;
      font-size: var(--fs-sm);
    "
  >
    <span
      class="shrink-0 truncate"
      style="
        width: 78px;
        color: var(--c-muted);
        font-weight: var(--fw-normal);
      "
    >
      {{ label }}
    </span>

    <span
      class="flex-1 min-w-0 flex items-center flex-wrap"
      style="gap: 6px; color: var(--c-foreground);"
    >
      <slot />
    </span>
  </div>
</template>
