<script setup lang="ts">
import { ref } from 'vue';

export interface DropdownOption {
  value: string;
  label: string;
  disabled?: boolean;
}

const props = withDefaults(
  defineProps<{
    options: DropdownOption[];
    modelValue?: string;
    placeholder?: string;
    disabled?: boolean;
  }>(),
  {
    modelValue: undefined,
    placeholder: 'Select…',
    disabled: false,
  },
);

const emit = defineEmits<{
  'update:modelValue': [value: string];
  change: [value: string];
}>();

const open = ref(false);

function selectOption(opt: DropdownOption) {
  if (opt.disabled) return;
  emit('update:modelValue', opt.value);
  emit('change', opt.value);
  open.value = false;
}

function toggle() {
  if (!props.disabled) open.value = !open.value;
}

const selectedLabel = () => {
  const found = props.options.find((o) => o.value === props.modelValue);
  return found ? found.label : props.placeholder;
};
</script>

<template>
  <div class="relative inline-block">
    <button
      type="button"
      class="inline-flex items-center gap-1 cursor-pointer select-none"
      :disabled="disabled"
      :style="`
        height: var(--h-button);
        padding: 0 8px;
        border-radius: var(--r-md);
        background-color: var(--c-raised);
        border: 1px solid var(--c-border);
        color: var(--c-foreground);
        font-family: var(--font-mono);
        font-size: var(--fs-sm);
        font-weight: var(--fw-medium);
        ${disabled ? 'opacity: 0.45; cursor: not-allowed;' : ''}
      `"
      @click="toggle"
    >
      <span>{{ selectedLabel() }}</span>
      <svg
        width="12"
        height="12"
        viewBox="0 0 12 12"
        fill="none"
        aria-hidden="true"
        :style="{ transform: open ? 'rotate(180deg)' : 'none', transition: 'transform 0.1s' }"
      >
        <path d="M3 4.5L6 7.5L9 4.5" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
      </svg>
    </button>

    <div
      v-if="open"
      class="absolute z-50 mt-1 min-w-full overflow-hidden"
      style="
        border-radius: var(--r-md);
        background-color: var(--c-panel);
        border: 1px solid var(--c-border);
        box-shadow: var(--shadow-md);
        top: 100%;
        left: 0;
      "
    >
      <ul role="listbox" style="list-style: none; padding: 2px 0;">
        <li
          v-for="opt in options"
          :key="opt.value"
          role="option"
          :aria-selected="opt.value === modelValue"
          :aria-disabled="opt.disabled"
          class="flex items-center px-3 cursor-pointer"
          :style="`
            height: var(--h-compact);
            font-size: var(--fs-sm);
            font-family: var(--font-mono);
            ${opt.disabled ? 'opacity: 0.45; cursor: not-allowed;' : ''}
            ${opt.value === modelValue ? 'background-color: var(--c-selection); color: var(--c-foreground);' : 'color: var(--c-foreground);'}
          `"
          @click="selectOption(opt)"
        >
          {{ opt.label }}
        </li>
      </ul>
    </div>
  </div>
</template>
