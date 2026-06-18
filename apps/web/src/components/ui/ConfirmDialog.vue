<script setup lang="ts">
import { onMounted, onUnmounted } from 'vue';
import Btn from '@/components/ui/Btn.vue';

/**
 * A blocking confirmation modal. The parent owns visibility via `open` and reacts
 * to `confirm` / `cancel`. Escape and backdrop clicks both cancel, so a dismissal
 * never reads as a confirmation.
 */
const props = withDefaults(
  defineProps<{
    open: boolean;
    title: string;
    message?: string;
    confirmLabel?: string;
    cancelLabel?: string;
    danger?: boolean;
  }>(),
  {
    confirmLabel: 'Confirm',
    cancelLabel: 'Cancel',
    danger: false,
  },
);

const emit = defineEmits<{
  confirm: [];
  cancel: [];
}>();

function onKeydown(event: KeyboardEvent): void {
  if (props.open && event.key === 'Escape') emit('cancel');
}

onMounted(() => window.addEventListener('keydown', onKeydown));
onUnmounted(() => window.removeEventListener('keydown', onKeydown));
</script>

<template>
  <Teleport to="body">
    <div
      v-if="open"
      class="fixed inset-0 flex items-center justify-center"
      style="background: rgba(0, 0, 0, 0.45); z-index: 300;"
      @mousedown.self="emit('cancel')"
    >
      <div
        role="dialog"
        aria-modal="true"
        :style="{
          width: '380px',
          maxWidth: 'calc(100vw - 32px)',
          background: 'var(--c-raised)',
          border: '1px solid var(--c-border)',
          borderRadius: 'var(--r-lg)',
          boxShadow: 'var(--shadow-lg)',
          padding: '18px 18px 16px',
          fontFamily: 'var(--font-ui)',
        }"
      >
        <h2
          style="font-size: var(--fs-md); font-weight: var(--fw-semibold); color: var(--c-foreground); margin: 0 0 8px;"
        >
          {{ title }}
        </h2>
        <p
          v-if="message"
          style="font-size: var(--fs-sm); color: var(--c-muted); line-height: 1.45; margin: 0 0 18px;"
        >
          {{ message }}
        </p>

        <div class="flex justify-end" style="gap: 8px;">
          <Btn variant="secondary" @click="emit('cancel')">{{ cancelLabel }}</Btn>
          <Btn
            v-if="!danger"
            variant="primary"
            @click="emit('confirm')"
          >
            {{ confirmLabel }}
          </Btn>
          <button
            v-else
            type="button"
            class="inline-flex items-center justify-center shrink-0 cursor-pointer select-none"
            style="height: var(--h-button); padding: 0 10px; border-radius: var(--r-md); font-family: var(--font-mono); font-size: var(--fs-sm); font-weight: var(--fw-medium); line-height: 1; background-color: var(--c-danger); color: var(--c-danger-fg, #fff); border: 1px solid transparent;"
            @click="emit('confirm')"
          >
            {{ confirmLabel }}
          </button>
        </div>
      </div>
    </div>
  </Teleport>
</template>
