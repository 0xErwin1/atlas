<script setup lang="ts">
import Icon from '@/components/ui/Icon.vue';
import { useOverlayEscape } from '@/composables/useOverlayEscape';

const props = withDefaults(
  defineProps<{
    open: boolean;
    title?: string;
    ariaLabel?: string;
  }>(),
  {
    title: '',
    ariaLabel: '',
  },
);

const emit = defineEmits<{
  close: [];
}>();

useOverlayEscape(
  () => props.open,
  () => emit('close'),
);
</script>

<template>
  <div
    v-if="open"
    data-sheet-backdrop
    class="fixed inset-0 flex flex-col justify-end"
    style="background-color: var(--c-overlay); z-index: 60;"
    @click.self="emit('close')"
  >
    <div
      role="dialog"
      :aria-label="ariaLabel || title || 'Sheet'"
      class="atl-sheet flex flex-col"
      style="
        width: 100%;
        max-height: 88vh;
        background-color: var(--c-panel);
        border-top: 1px solid var(--c-border);
        border-radius: var(--r-lg) var(--r-lg) 0 0;
        box-shadow: var(--shadow-lg);
        overflow: hidden;
      "
    >
      <div class="flex justify-center" style="padding: 8px 0 2px;" aria-hidden="true">
        <div style="width: 36px; height: 4px; border-radius: 9999px; background: var(--c-border);" />
      </div>

      <div
        v-if="title"
        class="flex items-center"
        style="gap: 10px; padding: 6px 14px 12px; border-bottom: 1px solid var(--c-border);"
      >
        <span
          class="flex-1 truncate"
          style="font-size: var(--fs-xl); font-weight: var(--fw-bold); color: var(--c-foreground);"
        >
          {{ title }}
        </span>
        <button
          type="button"
          data-action="close"
          title="Close"
          aria-label="Close"
          class="atl-gbtn"
          style="width: 26px; height: 26px;"
          @click="emit('close')"
        >
          <Icon name="x" :size="16" />
        </button>
      </div>

      <div class="flex-1 overflow-y-auto" style="padding: 14px;">
        <slot />
      </div>
    </div>
  </div>
</template>

<style scoped>
.atl-sheet {
  animation: atl-sheet-up 0.18s ease-out;
}

@keyframes atl-sheet-up {
  from {
    transform: translateY(8%);
    opacity: 0.6;
  }
  to {
    transform: translateY(0);
    opacity: 1;
  }
}
</style>
