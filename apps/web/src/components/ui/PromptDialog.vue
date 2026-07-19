<script setup lang="ts">
import { nextTick, ref, useId, watch } from 'vue';
import Btn from '@/components/ui/Btn.vue';
import DatePicker from '@/components/ui/DatePicker.vue';
import { useOverlayEscape } from '@/composables/useOverlayEscape';

/**
 * A single-field modal prompt. The parent owns visibility via `open` and reacts to
 * `confirm` (carrying the entered value) or `cancel`. Used for renaming and for
 * setting a date, so `inputType` switches between `text` and `date`. Escape and
 * backdrop clicks cancel without emitting a value.
 */
const props = withDefaults(
  defineProps<{
    open: boolean;
    title: string;
    initial?: string;
    placeholder?: string;
    confirmLabel?: string;
    inputType?: 'text' | 'date';
    error?: string;
  }>(),
  {
    initial: '',
    placeholder: '',
    confirmLabel: 'Save',
    inputType: 'text',
    error: '',
  },
);

const emit = defineEmits<{
  confirm: [value: string];
  cancel: [];
}>();

const value = ref(props.initial);
const inputRef = ref<HTMLInputElement | null>(null);
const dialogRef = ref<HTMLElement | null>(null);
const titleId = `atl-prompt-title-${useId()}`;
const errorId = `atl-prompt-error-${useId()}`;
let previouslyFocused: HTMLElement | null = null;

watch(
  () => props.open,
  async (open) => {
    if (open) {
      previouslyFocused = document.activeElement instanceof HTMLElement ? document.activeElement : null;
      value.value = props.initial;
      await nextTick();
      inputRef.value?.focus();
      inputRef.value?.select();
    } else if (previouslyFocused !== null) {
      await nextTick();
      if (previouslyFocused.isConnected) previouslyFocused.focus();
      previouslyFocused = null;
    }
  },
  { immediate: true },
);

function trapFocus(event: KeyboardEvent): void {
  if (event.key !== 'Tab' || dialogRef.value === null) return;

  const focusable = Array.from(
    dialogRef.value.querySelectorAll<HTMLElement>(
      'input:not([disabled]), button:not([disabled]), [href], [tabindex]:not([tabindex="-1"])',
    ),
  );
  if (focusable.length === 0) return;

  const first = focusable[0];
  const last = focusable[focusable.length - 1];
  if (event.shiftKey && document.activeElement === first) {
    event.preventDefault();
    last?.focus();
  } else if (!event.shiftKey && document.activeElement === last) {
    event.preventDefault();
    first?.focus();
  }
}

useOverlayEscape(
  () => props.open,
  () => emit('cancel'),
);
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
        ref="dialogRef"
        role="dialog"
        aria-modal="true"
        :aria-labelledby="titleId"
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
        @keydown="trapFocus"
      >
        <h2
          :id="titleId"
          style="font-size: var(--fs-md); font-weight: var(--fw-semibold); color: var(--c-foreground); margin: 0 0 12px;"
        >
          {{ title }}
        </h2>

        <DatePicker v-if="inputType === 'date'" v-model="value" />
        <input
          v-else
          ref="inputRef"
          v-model="value"
          :type="inputType"
          :placeholder="placeholder"
          :aria-invalid="error ? 'true' : undefined"
          :aria-describedby="error ? errorId : undefined"
          class="atl-prompt-input"
          @keydown.enter.prevent="emit('confirm', value)"
        />
        <p v-if="error" :id="errorId" role="alert" class="atl-prompt-error">{{ error }}</p>

        <div class="flex justify-end" style="gap: 8px; margin-top: 18px;">
          <Btn variant="secondary" @click="emit('cancel')">Cancel</Btn>
          <Btn variant="primary" @click="emit('confirm', value)">{{ confirmLabel }}</Btn>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.atl-prompt-input {
  width: 100%;
  height: 34px;
  padding: 0 10px;
  background: var(--c-panel);
  border: 1px solid var(--c-border);
  border-radius: var(--r-md);
  font-family: var(--font-ui);
  font-size: var(--fs-sm);
  color: var(--c-foreground);
  outline: none;
}

.atl-prompt-input:focus {
  border-color: var(--c-primary);
}

.atl-prompt-error {
  margin: 6px 0 0;
  color: var(--c-danger);
  font-size: var(--fs-xs);
}
</style>
