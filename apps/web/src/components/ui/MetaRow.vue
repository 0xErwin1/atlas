<script setup lang="ts">
defineProps<{
  label: string;
}>();
</script>

<template>
  <div
    class="flex items-center gap-2"
    style="
      min-height: var(--h-compact);
      font-size: var(--fs-sm);
      line-height: var(--lh-normal);
    "
  >
    <span
      class="shrink-0 truncate"
      style="
        width: 78px;
        color: var(--c-muted);
        font-size: var(--fs-sm);
        font-weight: var(--fw-normal);
      "
    >
      {{ label }}
    </span>

    <span class="flex-1 min-w-0 flex items-center gap-1 flex-wrap" style="color: var(--c-foreground)">
      <slot />
    </span>
  </div>
</template>
