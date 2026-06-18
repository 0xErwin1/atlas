<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, ref, watch } from 'vue';
import Btn from '@/components/ui/Btn.vue';

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
  }>(),
  {
    initial: '',
    placeholder: '',
    confirmLabel: 'Save',
    inputType: 'text',
  },
);

const emit = defineEmits<{
  confirm: [value: string];
  cancel: [];
}>();

const value = ref(props.initial);
const inputRef = ref<HTMLInputElement | null>(null);

watch(
  () => props.open,
  async (open) => {
    if (open) {
      value.value = props.initial;
      await nextTick();
      inputRef.value?.focus();
      inputRef.value?.select();
    }
  },
);

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
          style="font-size: var(--fs-md); font-weight: var(--fw-semibold); color: var(--c-foreground); margin: 0 0 12px;"
        >
          {{ title }}
        </h2>

        <input
          ref="inputRef"
          v-model="value"
          :type="inputType"
          :placeholder="placeholder"
          class="atl-prompt-input"
          @keydown.enter.prevent="emit('confirm', value)"
        />

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
</style>
