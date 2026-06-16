<script setup lang="ts">
import Icon from '@/components/ui/Icon.vue';

withDefaults(
  defineProps<{
    title: string;
    hint?: string;
    icon?: string;
  }>(),
  {
    hint: undefined,
    icon: 'file-text',
  },
);
</script>

<template>
  <div
    data-state="empty"
    class="flex flex-col items-center justify-center text-center"
    style="gap: 10px; padding: 24px; flex: 1; min-height: 0;"
  >
    <Icon :name="icon" :size="26" :style="{ color: 'var(--c-muted)' }" />
    <div style="font-size: 17px; font-weight: var(--fw-bold); color: var(--c-foreground);">
      {{ title }}
    </div>
    <div
      v-if="hint"
      style="font-size: var(--fs-base); color: var(--c-muted);"
    >
      {{ hint }}
    </div>
    <div
      v-if="$slots.actions"
      class="flex"
      style="gap: 8px; margin-top: 4px;"
    >
      <slot name="actions" />
    </div>
  </div>
</template>
