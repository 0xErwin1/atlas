<script setup lang="ts">
import Icon from '@/components/ui/Icon.vue';

withDefaults(
  defineProps<{
    label?: string;
  }>(),
  {
    label: 'Loading…',
  },
);
</script>

<template>
  <div
    data-state="loading"
    class="flex flex-col items-center justify-center text-center"
    style="gap: 10px; padding: 24px; flex: 1; min-height: 0;"
  >
    <span
      class="atl-spin inline-flex"
      :style="{ color: 'var(--c-muted)' }"
    >
      <Icon name="refresh-cw" :size="24" />
    </span>
    <div style="font-size: var(--fs-lg); color: var(--c-muted); font-family: var(--font-mono);">
      {{ label }}
    </div>
  </div>
</template>

<style scoped>
.atl-spin {
  animation: atl-spin 0.9s linear infinite;
}

@keyframes atl-spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
