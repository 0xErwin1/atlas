<script setup lang="ts">
const props = withDefaults(
  defineProps<{
    modelValue: boolean;
    label: string;
    disabled?: boolean;
    tone?: 'primary' | 'success';
  }>(),
  {
    disabled: false,
    tone: 'primary',
  },
);

const emit = defineEmits<{ 'update:modelValue': [value: boolean] }>();

defineOptions({ inheritAttrs: false });

function onChange(event: Event): void {
  if (!(event.target instanceof HTMLInputElement)) return;

  const requested = event.target.checked;
  emit('update:modelValue', requested);
  event.target.checked = props.modelValue;
}
</script>

<template>
  <input
    v-bind="$attrs"
    type="checkbox"
    class="atl-square-checkbox"
    :class="`atl-square-checkbox--${tone}`"
    :aria-label="label"
    :checked="modelValue"
    :disabled="disabled"
    data-square-checkbox
    @change="onChange"
  />
</template>

<style scoped>
.atl-square-checkbox {
  --atl-square-accent: var(--c-primary);
  --atl-square-check: var(--c-primary-fg);

  appearance: none;
  display: inline-grid;
  place-content: center;
  width: 15px;
  height: 15px;
  flex: 0 0 auto;
  margin: 0;
  padding: 0;
  border: 1px solid var(--c-muted);
  border-radius: var(--r-sm);
  background: var(--c-input);
  color: var(--atl-square-check);
  cursor: pointer;
}

.atl-square-checkbox::before {
  width: 7px;
  height: 4px;
  border-bottom: 2px solid currentColor;
  border-left: 2px solid currentColor;
  content: '';
  transform: rotate(-45deg) scale(0);
  transform-origin: center;
}

.atl-square-checkbox:checked {
  border-color: var(--atl-square-accent);
  background: var(--atl-square-accent);
}

.atl-square-checkbox:checked::before {
  transform: rotate(-45deg) scale(1);
}

.atl-square-checkbox:focus-visible {
  outline: none;
  border-color: var(--atl-square-accent);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--atl-square-accent) 20%, transparent);
}

.atl-square-checkbox:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.atl-square-checkbox--success {
  --atl-square-accent: var(--c-success);
  --atl-square-check: var(--c-background);
}
</style>
